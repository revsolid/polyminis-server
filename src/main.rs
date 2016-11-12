extern crate polyminis_core;

#[macro_use]
extern crate rustful;

use std::error::Error;

use rustful::{Context, Handler, Response, Server, TreeRouter};

#[macro_use]
extern crate log;
extern crate env_logger;

use std::sync::{Arc, RwLock};
use std::{thread, time};

struct SimulationMemory
{
    // Serialized steps
    steps: Vec<String>,
}

// This names are terrible (TODO)
struct ServiceMemory 
{
    simulations: Vec<SimulationMemory>
}



mod polymini_server_state 
{
    use std::sync::{Arc, RwLock};
    use std::{thread, time};
    use polyminis_core::control::*;
    use polyminis_core::environment::*;
    use polyminis_core::morphology::*;
    use polyminis_core::polymini::*;
    use polyminis_core::serialization::*;
    use polyminis_core::species::*;

    pub struct WorkerThreadState
    {
        kill: bool,
        steps: Vec<String>,
        static_state: String,
    }
    impl WorkerThreadState
    {
        fn new() -> WorkerThreadState
        {
            WorkerThreadState { kill: false, steps: vec![], static_state: "".to_string() }
        }
    
        fn worker_thread_main(workspace: Arc<RwLock<WorkerThreadState>>)
        { 
           let chromosomes = vec![[0, 0x09, 0x6A, 0xAD],
                                  [0, 0x0B, 0xBE, 0xDA],
                                  [0,    0, 0xBE, 0xEF],
                                  [0,    0, 0xDB, 0xAD]];

            let p1 = Polymini::new(Morphology::new(chromosomes, &TranslationTable::new()),
                                   Control::new());
            let mut sim = Simulation::new();
            sim.add_species(Species::new(vec![p1]));
            sim.add_object((10.0, 2.0), (1, 1));

            // Critical Section
            {
                let mut w = workspace.write().unwrap();
                w.static_state = sim.serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_STATIC)).to_string();
            }
            
            //TODO: Maybe some set of stopping criteria
            while true
            {
                sim.step();
                // Critical Section
                {
                    let mut w = workspace.write().unwrap();
    
                    if w.kill
                    {
                        break;
                    }
    
                    let step_string = sim.serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DYNAMIC)).to_string();
                    w.steps.push(step_string);
                }

                let five_s = time::Duration::from_millis(5000);
                thread::sleep(five_s); 
            }
        }
    }

    #[derive(Clone)]
    pub struct SimulationState
    {
        static_state: Option<String>,
        work_thread_state: Arc<RwLock<WorkerThreadState>>,
    }
    impl SimulationState
    {
        pub fn get_static_state(&self) -> &Option<String>
        {
            &self.static_state
        }
        pub fn get_or_cache_static_state(&mut self) -> &Option<String>
        {
            match self.static_state
            {
                Some(_) => {}, 
                None =>
                {
                    let w = self.work_thread_state.read().unwrap();
                    self.static_state = Some(w.static_state.clone());
                }
            }
            &self.static_state
        }
        pub fn get_dynamic_state(&self) -> Vec<String>
        {
            let result: Vec<String>;

            // Critical Section
            {
                let ws = self.work_thread_state.read().unwrap();
                result = ws.steps.clone();
            }

            result
        }
    }

    #[derive(Clone)]
    pub struct ServerState
    {
        pub simulations: Vec<SimulationState>,
    }
    impl ServerState
    {
        pub fn new() -> ServerState
        {
            ServerState { simulations: vec![] }
        }

        pub fn add_simulation(&mut self)
        {

            let workspace = Arc::new(RwLock::new(WorkerThreadState::new()));

            let simulation_state = SimulationState { static_state: None,
                                                     work_thread_state: workspace.clone() };
            self.simulations.push(simulation_state);

            let thread_copy = workspace.clone();
            thread::spawn(move ||
            { 
                WorkerThreadState::worker_thread_main(thread_copy);
            });
        }

        pub fn get_simulation_state_by_inx(&self, i: usize) -> (String, Vec<String>)
        {
            if  i < self.simulations.len()
            {
                (self.simulations[i].get_static_state().clone().unwrap(), self.simulations[i].get_dynamic_state())
            }
            else
            {
                ("".to_string(), vec![])
            }
        }
    }
}


mod polymini_server_endpoints 
{
    use rustful::{Context, Handler, Response, Server, TreeRouter};
    use ::polymini_server_state::{ServerState, SimulationState};

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
        SimulationStateAll         { s: ServerState },
        SimulationStateOne         { s: ServerState },

            EpochStateAll          { s: ServerState },
            EpochStateOne          { s: ServerState },

                StepStateAll       { s: ServerState },
                StepStateOne       { s: ServerState },
    }
    impl Handler for Simulation
    {
        fn handle_request(&self, context: Context, mut response: Response)
        {

            let mut simulation_num = None;
            if let Some(simnumber) = context.variables.get("simnumber")
            {
                let sim_num_result = usize::from_str_radix(&simnumber, 10);
                match sim_num_result
                {
                    Ok (sim_num) =>
                    {
                        simulation_num = Some(sim_num);
                    },
                    Err(e) =>
                    {
                        response.send(format!("{:?}", e));
                        return
                    }
                }
            }

            let mut step_num = None;
            if let Some(step) = context.variables.get("step")
            {
                let step_inx_result = usize::from_str_radix(&step, 10);
                match step_inx_result
                {
                    Ok(s_inx) =>
                    {
                        step_num = Some(s_inx);
                    },
                    Err(e) =>
                    {
                        response.send(format!("{:?}", e));
                        return
                    }
                }
            }


            match *self
            {
                //TODO:
                Simulation::SimulationStateAll {ref s } => {},
                Simulation::SimulationStateOne { ref s } => {},
                Simulation::EpochStateAll {ref s } => {},
                Simulation::EpochStateOne { ref s } => {},
                //~TODO:
                
                Simulation::StepStateAll { ref s } =>
                {
                    if let Some(sim_num) = simulation_num
                    {
                        let sim = &s.simulations[sim_num];
                        let mut simulation_dump: String = "".to_string();
                        for ss in &sim.get_dynamic_state()
                        {
                            simulation_dump = simulation_dump + ss;
                        }
                        response.send(simulation_dump);
                    }
                },
                Simulation::StepStateOne { ref s } =>
                {
                    if let Some(sim_num) = simulation_num
                    {
                        let sim = &s.simulations[sim_num];
                        let steps = sim.get_dynamic_state();

                        if let Some(step_n) = step_num
                        {
                            if steps.len() < step_n
                            {
                                response.send(format!("Step {} not ready", step_n));
                            }
                            else
                            {
                                let step_string = sim.get_dynamic_state()[step_n - 1].clone();
                                response.send(step_string);
                            }
                        }
                    }
                },
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

    let mut ss = ServerState::new();

    ss.add_simulation();


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
                    Get: Endpoint::Simulation(Simulation::SimulationStateAll { s: ss.clone() }),
                    ":simnumber" =>
                    {
                        Get: Endpoint::Simulation(Simulation::SimulationStateOne { s: ss.clone() }),
                        "epochs" =>
                        {
                            Get: Endpoint::Simulation(Simulation::EpochStateAll { s: ss.clone() }),
                            ":epoch" => 
                            {
                                Get: Endpoint::Simulation(Simulation::EpochStateOne { s: ss.clone() }),
                                "steps" =>
                                {
                                    Get: Endpoint::Simulation(Simulation::StepStateAll{ s: ss.clone() }),
                                    ":step" => Get: Endpoint::Simulation(Simulation::StepStateOne{ s: ss.clone() }),
                                },
                                /* TODO:
                                "species" => {}
                                */
                            }
                        },
                        /* TODO:
                        "environment" => {}
                        */
                    }
                }
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


//TODO: This might be better off somewhere else
// On a separate note, this is an AMAZING way of doing variable binding :O
enum Api
{
    TestSim
    {
        service_memory: Arc<RwLock<ServiceMemory>>
    },
    TestEmpty
}
impl Handler for Api
{
    fn handle_request(&self, context: Context, mut response: Response)
    {
        match *self
        {
            _ => {},
        };
    }
}
