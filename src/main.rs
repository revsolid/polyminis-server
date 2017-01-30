extern crate lru_cache;

extern crate polyminis_core;

#[macro_use]
extern crate rustful;
use rustful::{Context, Handler, Response, Server, TreeRouter};

extern crate env_logger;
#[macro_use]
extern crate log;

pub use std::error::Error;
pub use std::sync::{Arc, RwLock};
pub use std::thread;

mod slave;

mod polymini_server_state
{
    extern crate polyminis_core;
    use super::*;
    use super::slave::*;

    use polyminis_core::serialization::*;

    #[derive(Clone)]
    pub struct SimulationState
    {
        work_thread_state: Arc<RwLock<WorkerThreadState>>,
    }
    impl SimulationState
    {
        pub fn new(worker_thread_state: &Arc<RwLock<WorkerThreadState>>) -> SimulationState
        {
            SimulationState { work_thread_state: worker_thread_state.clone() }
        }

        pub fn serialize(&self) -> Json 
        {
            let mut json_obj = pmJsonObject::new();
            {
                let ws = self.work_thread_state.read().unwrap();

            }
            Json::Object(json_obj)
        }

        pub fn serialize_epoch_all(&self) -> Json
        {
            let mut json_arr = pmJsonArray::new();

            // Critical Section
            {
                let ws = self.work_thread_state.read().unwrap();
                for (k,v) in &ws.epochs
                {
                    json_arr.push(v.serialize());
                }
            }

            Json::Array(json_arr)
        }

        pub fn serialize_epoch(&self, e: usize) -> Json
        {
            let json;
            {
                let mut ws = self.work_thread_state.write().unwrap();
                json = ws.epochs.get_mut(&(e as u32)).unwrap().serialize();
            }
            json
        }

        pub fn serialize_species_all(&self, e: usize) -> Json
        {
            let mut json_obj = pmJsonObject::new();
            let mut json_arr = pmJsonArray::new();
            {
                let mut ws = self.work_thread_state.write().unwrap();
                for s in &ws.epochs.get_mut(&(e as u32)).unwrap().species
                {
                    json_arr.push(s.clone());
                }
            }
            json_obj.insert("Species".to_owned(), Json::Array(json_arr));
            Json::Object(json_obj)
        }

        pub fn serialize_environment(&self, e: usize) -> Json
        {
            let json;
            {
                let mut ws = self.work_thread_state.write().unwrap();
                json = ws.epochs.get_mut(&(e as u32)).unwrap().environment.clone();
            }
            Json::Object(json)
        }

        pub fn serialize_step_all(&self, e: usize) -> Json
        {
            let mut json_obj = pmJsonObject::new();
            let mut json_arr = pmJsonArray::new();
            {
                let mut ws = self.work_thread_state.write().unwrap();
                trace!("XXX");
                for s in &ws.epochs.get_mut(&(e as u32)).unwrap().steps
                {
                    json_arr.push(s.clone());
                }
            }
            json_obj.insert("Steps".to_owned(), Json::Array(json_arr));
            Json::Object(json_obj)

        }
        pub fn serialize_step(&self, e: usize, s: usize) -> Json
        {
            let json;
            {
                let mut ws = self.work_thread_state.write().unwrap();
                json = ws.epochs.get_mut(&(e as u32)).unwrap().steps[s].clone();
            }
            json
        }

        pub fn serialize_persistent_data(&self, e: usize) -> Json
        {
            let json;
            {
                let mut ws = self.work_thread_state.write().unwrap();
                json = Json::Object(ws.epochs.get_mut(&(e as u32)).unwrap().persistable_data.clone());
            }
            json
        }

        pub fn serialize_persistent_data_species(&self, e: usize) -> Json
        {
            let json;
            {
                let mut ws = self.work_thread_state.write().unwrap();
                json = Json::Object(ws.epochs.get_mut(&(e as u32)).unwrap().persistable_species_data.clone());
            }
            json

        }

        pub fn advance(&mut self, data: Json) -> Json
        {
            let mut json_obj = pmJsonObject::new();
            {
                let mut ws = self.work_thread_state.write().unwrap();
                ws.actions.push_back(WorkerThreadActions::Advance { simulation_data: data });
                match ws.thread_hndl
                {
                    Some(ref hdl) =>
                    {
                        hdl.unpark();
                    }
                    None => {}
                }
            }
            Json::Object(json_obj)
        }

        pub fn simulate(&mut self, data: Json) -> Json
        {
            let mut json_obj = pmJsonObject::new();
            {
                let mut ws = self.work_thread_state.write().unwrap();
                ws.actions.push_back(WorkerThreadActions::Simulate { simulation_data: data });
                match ws.thread_hndl
                {
                    Some(ref hdl) =>
                    {
                        hdl.unpark();
                    }
                    None => {}
                }
            }
            Json::Object(json_obj)
        }
    }

    #[derive(Clone)]
    pub struct ServerState
    {
        pub simulations: Arc<RwLock<Vec<SimulationState>>>,
    }
    impl ServerState
    {
        pub fn new() -> ServerState
        {
            ServerState { simulations: Arc::new(RwLock::new(vec![])) }
        }

        pub fn add_simulation(&self)
        {
            let wst = WorkerThreadState::new();

            let workspace = Arc::new(RwLock::new(wst));

            let mut simulation_state = SimulationState::new(&workspace);

            {
                self.simulations.write().unwrap().push(simulation_state);
            }

            let thread_copy = workspace.clone();
            thread::spawn(move ||
            {
                WorkerThreadState::worker_thread_main(thread_copy);
            });
        }

        pub fn serialize(&self) -> Json
        {
            let mut json_obj = pmJsonObject::new();

            json_obj.insert("NumSimulations".to_owned(), self.simulations.read().unwrap().len().to_json());

            Json::Object(json_obj)
        }
    }
}

mod polymini_server_endpoints
{
    use rustful::{Context, Handler, Response, Server, TreeRouter};
    use polymini_server_state::{ServerState, SimulationState};

    #[derive(Debug, Clone, Copy)]
    enum Error
    {
        NotFound,
        InternalServerError,
    }

    struct EndpointHelper;
    impl EndpointHelper
    {
        //TODO: Maybe make it templatized?
        pub fn get_usize_var(context: &Context,name: &str) -> Option<usize>
        {
            if let Some(number) = context.variables.get(name)
            {
                let num_result = usize::from_str_radix(&number, 10);
                match num_result
                {
                    Ok (r) =>
                    {
                        Some(r)
                    },
                    Err(e) =>
                    {
                        None
                    }
                }
            }
            else
            {
                None
            }
        }

        pub fn send_error(err: Error, mut response: Response)
        {
            match err
            {
                Error::NotFound =>
                {
                    response.send("Error Not Found");
                },
                Error::InternalServerError =>
                {
                    response.send("Internal Error");
                },
            }
        }
    }

    pub enum Endpoint
    {
        Simulation(Simulation),
        Management(Management),
    }
    impl Handler for Endpoint
    {
        fn handle_request(&self, context: Context, mut response: Response)
        {
            match *self
            {
                Endpoint::Simulation(ref simEndpoint) => { simEndpoint.handle_request(context, response); },
                Endpoint::Management(ref mgtEndpoint) => { mgtEndpoint.handle_request(context, response); },
            }
        }
    }

    pub enum Simulation
    {
        SimulationStateAll               { s: ServerState },
        SimulationStateOne               { s: ServerState },
        AddSimulation                    { s: ServerState },

            EpochStateAll                { s: ServerState },
            EpochStateOne                { s: ServerState },

                AdvanceEpoch             { s: ServerState },
                SimulateEpoch            { s: ServerState },

                StepStateAll             { s: ServerState },
                StepStateOne             { s: ServerState },

                SpeciesInEpoch           { s: ServerState },
                SpeciesInEpochOne        { s: ServerState },

                EnvironmentInEpoch       { s: ServerState },

                PersistentDataEpoch      { s: ServerState },
                PersistentDataSpecies    { s: ServerState },
                PersistentDataSpeciesOne { s: ServerState },
    }

    impl Handler for Simulation
    {
        fn handle_request(&self, mut context: Context, mut response: Response)
        {

            let simulation_num = EndpointHelper::get_usize_var(&context, "simnumber");
            let epoch_num = EndpointHelper::get_usize_var(&context, "epoch");
            let step_num = EndpointHelper::get_usize_var(&context, "step");


            // /simulations/
            match *self
            {
                Simulation::SimulationStateAll {ref s } =>
                {
                    // This should return information about all
                    // the simulations in the server.
                     response.send(s.serialize().to_string());
                     return;
                },
                Simulation::SimulationStateOne { ref s } =>
                { 
                    // This should return information about this particular simulation;
                    if let Some(sim_inx) = simulation_num 
                    {
                        let sim = &s.simulations.read().unwrap()[sim_inx];
                        response.send(sim.serialize().to_string());
                    }
                    else
                    {
                        EndpointHelper::send_error(Error::NotFound, response);
                    }
                    return;
                },
                Simulation::AddSimulation { ref s } =>
                {
                    //TODO: Pass data for new simulation
                    s.add_simulation();
                }
                _ => {},
            }


            let mut simulation_index = match simulation_num
            {
                Some(s_i) => { s_i }
                None =>  {  EndpointHelper::send_error(Error::InternalServerError, response); return; }
            };

            // /simulations/simulation_num/epochs
            match *self
            {
                Simulation::EpochStateAll { ref s } =>
                {
                    // Return all the Epochs for a particular simulation
                    let sim = &s.simulations.read().unwrap()[simulation_index];
                    response.send(sim.serialize_epoch_all().to_string());
                    return;
                },

                Simulation::EpochStateOne { ref s } =>
                {
                    // Return the state for a particular Epoch in a particular simulation
                    if let Some(e_num) = epoch_num
                    {
                        let sim = &s.simulations.read().unwrap()[simulation_index];
                        response.send(sim.serialize_epoch(e_num).to_string());
                        return;
                    }
                    else
                    {
                        EndpointHelper::send_error(Error::NotFound, response);
                    }
                    return;
                },
                _ => {},
            }

            // Actions
            // /simulations/simulation_num/epochs/
            match *self
            {
                Simulation::AdvanceEpoch { ref s } =>
                {
                    let sim = &mut s.simulations.write().unwrap()[simulation_index];
                    let json = context.body.read_json_body().unwrap();
                    sim.advance(json);
                    return;
                },
                Simulation::SimulateEpoch { ref s } =>
                {
                    let sim = &mut s.simulations.write().unwrap()[simulation_index];
                    let json = context.body.read_json_body().unwrap();
                    sim.simulate(json);
                    return;
                },
                _ => {},
            }


            let mut epoch = match epoch_num
            {
                Some(e_n) => { e_n }
                None =>  {  EndpointHelper::send_error(Error::InternalServerError, response); return; }
            };
 
            // /simulations/simulation_num/epochs/epoch_num/species
            // /simulations/simulation_num/epochs/epoch_num/environment
            match *self
            {
                Simulation::SpeciesInEpoch { ref s } =>
                {
                    let sim = &s.simulations.read().unwrap()[simulation_index];
                    response.send(sim.serialize_species_all(epoch).to_string());
                    return;
                },
                Simulation::SpeciesInEpochOne { ref s } =>
                {
                },
                Simulation::EnvironmentInEpoch { ref s } =>
                {
                    let sim = &s.simulations.read().unwrap()[simulation_index]; 
                    response.send(sim.serialize_environment(epoch).to_string());
                    return;
                },
                _ => {}
            }

            // /simulations/simulation_num/epochs/epoch_num/steps
            match *self
            {
                Simulation::StepStateAll { ref s } =>
                {
                    let sim = &s.simulations.read().unwrap()[simulation_index];
                    response.send(sim.serialize_step_all(epoch).to_string());
                    return;
                },
                Simulation::StepStateOne { ref s } =>
                {
                    if let Some(step_n) = step_num
                    {
                        let sim = &s.simulations.read().unwrap()[simulation_index];
                        response.send(sim.serialize_step(epoch, step_n).to_string());
                    }
                    else
                    {
                        EndpointHelper::send_error(Error::NotFound, response);
                    }
                    return;
                },
                _ => {}
            }

            // /simulations/simulation_num/epochs/epoch_num/db
            match *self
            {
                Simulation::PersistentDataEpoch { ref s } =>
                {
                    let sim = &s.simulations.read().unwrap()[simulation_index];
                    response.send(sim.serialize_persistent_data(epoch).to_string());
                },
                Simulation::PersistentDataSpecies { ref s } =>
                {
                    let sim = &s.simulations.read().unwrap()[simulation_index];
                    response.send(sim.serialize_persistent_data_species(epoch).to_string());
                },
                _ => {}
            }

        }
    }

    pub enum Management
    {
        Ping {}
    }
    impl Handler for Management
    {
        fn handle_request(&self, context: Context, mut response: Response)
        {
            match *self
            {
                _ => { response.send("MGMT PING"); }
            }
        }
    }
}

fn main()
{
    use ::polymini_server_state::*;
    use ::polymini_server_endpoints::*;

    extern crate env_logger;

    let mut ss = ServerState::new();


    ss.add_simulation();

    let _ = env_logger::init();

    //Build and run the server.
    let server_result = Server {
        //Turn a port number into an IPV4 host address (0.0.0.0:8080 in this case).
        host: 8080.into(),

        //Create a TreeRouter and fill it with handlers.
        handlers: insert_routes!
        {
            TreeRouter::new() =>
            {
                Get: Endpoint::Management(Management::Ping{}),
                "simulations" =>
                {
                    Get:  Endpoint::Simulation(Simulation::SimulationStateAll { s: ss.clone() }),
                    "add" =>
                    {
                        Post: Endpoint::Simulation(Simulation::AddSimulation { s: ss.clone() }),
                    },
                    ":simnumber" =>
                    {
                        Get: Endpoint::Simulation(Simulation::SimulationStateOne { s: ss.clone() }),
                        "epochs" =>
                        {
                            Get: Endpoint::Simulation(Simulation::EpochStateAll { s: ss.clone() }),
                            "advance" =>
                            {
                                Post: Endpoint::Simulation(Simulation::AdvanceEpoch { s: ss.clone() }),
                            },
                            "simulate" =>
                            {
                                Post: Endpoint::Simulation(Simulation::SimulateEpoch { s: ss.clone() }),
                            },
                            ":epoch" =>
                            {
                                Get: Endpoint::Simulation(Simulation::EpochStateOne { s: ss.clone() }),
                                "steps" =>
                                {
                                    Get: Endpoint::Simulation(Simulation::StepStateAll{ s: ss.clone() }),
                                    ":step" => Get: Endpoint::Simulation(Simulation::StepStateOne{ s: ss.clone() }),
                                },
                                "species" =>
                                {
                                    Get: Endpoint::Simulation( Simulation::SpeciesInEpoch  { s: ss.clone() } ),
                                    ":species" => Get: Endpoint::Simulation( Simulation::SpeciesInEpochOne { s: ss.clone() }),
                                },
                                "environment" =>
                                {
                                    Get: Endpoint::Simulation( Simulation::EnvironmentInEpoch { s: ss.clone() } ),
                                },
                                "db" =>
                                {
                                    Get: Endpoint::Simulation( Simulation::PersistentDataEpoch { s: ss.clone() } ),
                                    "species" =>
                                    {
                                        Get: Endpoint::Simulation( Simulation::PersistentDataSpecies { s: ss.clone() } ),
                                        ":species" => Get: Endpoint::Simulation( Simulation::PersistentDataSpeciesOne { s: ss.clone() } ),
                                    },
                                },
                            },
                        },
                    },
                },
            }
        },

        //Use default values for everything else.
        ..Server::default()
    }.run();

    match server_result
    {
        Ok(_server) => {},
        Err(e) => error!("could not start server: {}", e.description())
    }
}
