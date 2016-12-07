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

mod polymini_server_state
{
    use std::sync::{Arc, RwLock};
    use std::{thread, time};
    use std::collections::{HashMap, HashSet};

    use polyminis_core::actuators::*;
    use polyminis_core::environment::*;
    use polyminis_core::evaluation::*;
    use polyminis_core::genetics::*;
    use polyminis_core::morphology::*;
    use polyminis_core::polymini::*;
    use polyminis_core::sensors::*;
    use polyminis_core::serialization::*;
    use polyminis_core::simulation::*;
    use polyminis_core::species::*;
    use polyminis_core::traits::*;
    use ::env_logger;

    pub struct WorkerThreadState
    {
        kill: bool,
        simulation_state: String,
        epochs: Vec<EpochState>,
    }
    impl WorkerThreadState
    {
        fn new() -> WorkerThreadState
        {
            WorkerThreadState { kill: false, epochs: vec![], simulation_state: "".to_string() }
        }

        fn worker_thread_main(_: Json,
                              workspace: Arc<RwLock<WorkerThreadState>>)
        {

            let mut sim = Simulation::new(); 
            let mut master_translation_table = HashMap::new();

            master_translation_table.insert( (TraitTier::TierI, 2), PolyminiTrait::PolyminiSimpleTrait(PolyminiSimpleTrait::SpeedTrait));
            master_translation_table.insert( (TraitTier::TierI, 1), PolyminiTrait::PolyminiActuator(ActuatorTag::MoveHorizontal));
            master_translation_table.insert( (TraitTier::TierI, 3), PolyminiTrait::PolyminiActuator(ActuatorTag::MoveVertical));
            master_translation_table.insert( (TraitTier::TierI, 4), PolyminiTrait::PolyminiActuator(ActuatorTag::MoveHorizontal));
            master_translation_table.insert( (TraitTier::TierI, 5), PolyminiTrait::PolyminiActuator(ActuatorTag::MoveVertical));

            let mut active_table_1 = HashSet::new();
            active_table_1.insert( (TraitTier::TierI, 5) );
            active_table_1.insert( (TraitTier::TierI, 4) );
            active_table_1.insert( (TraitTier::TierI, 3) );
            active_table_1.insert( (TraitTier::TierI, 2) );
            active_table_1.insert( (TraitTier::TierI, 1) );

            let mut active_table_2 = HashSet::new();
            active_table_2.insert( (TraitTier::TierI, 3) );
            active_table_2.insert( (TraitTier::TierI, 2) );
            active_table_2.insert( (TraitTier::TierI, 1) );


            let default_sensors = vec![ Sensor::new(SensorTag::PositionX, 1),
                                        Sensor::new(SensorTag::PositionY, 1),
                                        Sensor::new(SensorTag::Orientation, 1),
                                        Sensor::new(SensorTag::LastMoveSucceded, 1)];

            let evaluators = vec![ FitnessEvaluator::OverallMovement { weight: 0.5 },
                                   FitnessEvaluator::DistanceTravelled { weight: 2.0 },
                                   FitnessEvaluator::Shape { weight: 15.0 },
                                   FitnessEvaluator::Alive { weight: 15.0 },
                                   FitnessEvaluator::PositionsVisited { weight: 5.0 },
                                   FitnessEvaluator::TargetPosition { weight: 15.0, pos: (0.8, 0.8) },
                                
                                   ];

            let translation_table_species_1 = TranslationTable::new_from(&master_translation_table, &active_table_1);
            let translation_table_species_2 = TranslationTable::new_from(&master_translation_table, &active_table_2);


            let mut env = Environment::new(2, default_sensors);


            env.add_static_object((0.0, 0.0),  (100, 1));
            env.add_static_object((0.0, 0.0),  (1, 100));
            env.add_static_object((99.0, 0.0), (1, 100));
            env.add_static_object((0.0, 99.0), (100, 1));
            env.add_object(WorldObject::new_static_object((50.0, 15.0), (5, 10)));
            env.add_object(WorldObject::new_static_object((50.0, 45.0), (5, 10)));
            env.add_object(WorldObject::new_static_object((50.0, 75.0), (5, 10)));
            env.add_object(WorldObject::new_static_object((15.0, 50.0), (10, 5)));
            env.add_object(WorldObject::new_static_object((45.0, 50.0), (10, 5)));
            env.add_object(WorldObject::new_static_object((75.0, 50.0), (10, 5)));

            let gens_per_epoch = 100;

            let cfg = PGAConfig { max_generations: gens_per_epoch, population_size: 50,
                                  percentage_elitism: 0.2, percentage_mutation: 0.25, fitness_evaluators: evaluators, genome_size: 8 };

            trace!("Creating Species");
            let s1 = Species::new_from("Test Species 1".to_owned(), translation_table_species_1,
                                       &env.default_sensors, cfg.clone(),
                                       Box::new(| ctx: &mut PolyminiRandomCtx |
                                       {
                                           ((ctx.gen_range(0.0, 45.0) as f32).floor(),
                                            (ctx.gen_range(0.0, 45.0) as f32).floor())
                                       }));
            //let s2 = Species::new_from("Test Species 2".to_owned(), translation_table_species_2,
            //                           &env.default_sensors, cfg.clone());


            trace!("Adding Species");
            let mut epoch = SimulationEpoch::new_restartable(env, gens_per_epoch as usize, 1);
            epoch.add_species(s1);
            //epoch.add_species(s2);
            
            trace!("Swaping Species:");
            sim.swap_epoch(epoch);

            let total_epochs = 150;

            let mut kill = false;
            let mut logger;
            logger = env_logger::init();
            'outer: for i in 0..total_epochs
            {

                {
                    let mut w = workspace.write().unwrap();
                    let mut epoch_state = EpochState::new(i);
                    let mut ctx = SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_STATIC);

                    for s in sim.get_epoch().get_species() 
                    {
                        epoch_state.species.push(s.serialize(&mut ctx));
                    }
                    
                    let mut env_obj = pmJsonObject::new();
                    env_obj.insert("Environment".to_owned(), sim.get_epoch().get_environment().serialize(&mut ctx));
                    epoch_state.environment = env_obj;
                    w.epochs.push(epoch_state);
                }
                loop
                {
                    let break_loop = sim.step();
                    let mut w = workspace.write().unwrap();

                    if w.kill
                    {
                        kill = true;
                        break 'outer;
                    }

                    let step_json = sim.get_epoch().serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DYNAMIC));
                    w.epochs[i].steps.push(step_json);

                    if break_loop
                    {
                        break;
                    }
                }

                sim.get_epoch_mut().evaluate_species(); 
                {
                    let mut w = workspace.write().unwrap();
                    w.epochs[i].evaluated = true;

                    for s in sim.get_epoch().get_species()
                    {
                        w.epochs[i].evaluation_data.insert(s.get_name().clone(), s.get_best().serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_STATS)));
                    }
                }



                if i < total_epochs - 1
                {
                    sim.advance_epoch();
                }
            }

            if kill
            {
                // Kill simulation
            }
        }
    }


    #[derive(Clone)]
    pub struct EpochState
    {
        epoch_num: usize,
        steps: pmJsonArray,
        species: pmJsonArray,
        evaluated: bool,
        evaluation_data: pmJsonObject,
        environment: pmJsonObject,
    }
    impl EpochState
    {
        pub fn new(i: usize) -> EpochState
        {
            EpochState { epoch_num: i, steps: pmJsonArray::new(),  species: pmJsonArray::new(), evaluated: false, evaluation_data: pmJsonObject::new(),
                         environment: pmJsonObject::new() }
        }
        pub fn serialize(&self) -> Json
        {
            let mut json_obj = pmJsonObject::new();
            json_obj.insert("Epoch".to_owned(),     self.epoch_num.to_json());
            json_obj.insert("Steps".to_owned(),     self.steps.len().to_json());
            json_obj.insert("Evaluated".to_owned(), self.evaluated.to_json());

            if (self.evaluated)
            {
                json_obj.insert("EvaluationData".to_owned(), Json::Object(self.evaluation_data.clone()));
            }
            Json::Object(json_obj)
        }
    }
    
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
                for e in &ws.epochs
                {
                    json_arr.push(e.serialize());
                }
            }

            Json::Array(json_arr)
        }

        pub fn serialize_epoch(&self, e: usize) -> Json
        {
            let json;
            {
                let ws = self.work_thread_state.read().unwrap();
                json = ws.epochs[e].serialize();
            }
            json
        }

        pub fn serialize_species_all(&self, e: usize) -> Json
        {
            let mut json_obj = pmJsonObject::new();
            let mut json_arr = pmJsonArray::new();
            {
                let ws = self.work_thread_state.read().unwrap();
                for s in &ws.epochs[e].species
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
                let ws = self.work_thread_state.read().unwrap();
                json = ws.epochs[e].environment.clone();
            }
            Json::Object(json)
        }

        pub fn serialize_step_all(&self, e: usize) -> Json
        {
            let mut json_obj = pmJsonObject::new();
            let mut json_arr = pmJsonArray::new();
            {
                let ws = self.work_thread_state.read().unwrap();
                for s in &ws.epochs[e].steps
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
                let ws = self.work_thread_state.read().unwrap();
                json = ws.epochs[e].steps[s].clone();
            }
            json
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

            let mut simulation_state = SimulationState::new(&workspace);

            self.simulations.push(simulation_state);

            let thread_copy = workspace.clone();
            thread::spawn(move ||
            {
                WorkerThreadState::worker_thread_main(Json::Object(pmJsonObject::new()), thread_copy);
            });
        }

        pub fn serialize(&self) -> Json
        {
            let mut json_obj = pmJsonObject::new();

            json_obj.insert("NumSimulations".to_owned(), self.simulations.len().to_json());

            Json::Object(json_obj)
        }
    }
}


mod polymini_server_endpoints
{
    use rustful::{Context, Handler, Response, Server, TreeRouter};
    use ::polymini_server_state::{ServerState, SimulationState};

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
        SimulationStateAll         { s: ServerState },
        SimulationStateOne         { s: ServerState },

            EpochStateAll          { s: ServerState },
            EpochStateOne          { s: ServerState },

                StepStateAll       { s: ServerState },
                StepStateOne       { s: ServerState },

                SpeciesInEpoch     { s: ServerState },
                SpeciesInEpochOne  { s: ServerState },

                EnvironmentInEpoch { s: ServerState },
    }

    impl Handler for Simulation
    {
        fn handle_request(&self, context: Context, mut response: Response)
        {

            let simulation_num = EndpointHelper::get_usize_var(&context, "simnumber");
            let epoch_num = EndpointHelper::get_usize_var(&context, "epoch");
            let step_num = EndpointHelper::get_usize_var(&context, "step");

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
                        let sim = &s.simulations[sim_inx];
                        response.send(sim.serialize().to_string());
                    }
                    else
                    {
                        EndpointHelper::send_error(Error::NotFound, response);
                    }
                    return;
                },
                _ => {},
            }


            let mut simulation_index = match simulation_num
            {
                Some(s_i) => { s_i }
                None =>  {  EndpointHelper::send_error(Error::InternalServerError, response); return; unreachable!(); }
            };

            match *self
            {
                Simulation::EpochStateAll { ref s } =>
                {
                    // Return all the Epochs for a particular simulation
                    let sim = &s.simulations[simulation_index];
                    response.send(sim.serialize_epoch_all().to_string());
                    return;
                },

                Simulation::EpochStateOne { ref s } =>
                {
                    // Return the state for a particular Epoch in a particular simulation
                    if let Some(e_num) = epoch_num
                    {
                        let sim = &s.simulations[simulation_index];
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

            let mut epoch = match epoch_num
            {
                Some(e_n) => { e_n }
                None =>  {  EndpointHelper::send_error(Error::InternalServerError, response); return; }
            };



            match *self
            {
                Simulation::SpeciesInEpoch { ref s } =>
                {
                    let sim = &s.simulations[simulation_index];
                    response.send(sim.serialize_species_all(epoch).to_string());
                    return;
                },
                Simulation::SpeciesInEpochOne { ref s } =>
                {
                },
                Simulation::EnvironmentInEpoch { ref s } =>
                {
                    let sim = &s.simulations[simulation_index]; 
                    response.send(sim.serialize_environment(epoch).to_string());
                    return;
                },
                _ => {}
            }

            match *self
            {
                Simulation::StepStateAll { ref s } =>
                {
                    let sim = &s.simulations[simulation_index];
                    response.send(sim.serialize_step_all(epoch).to_string());
                    return;
                },
                Simulation::StepStateOne { ref s } =>
                {
                    if let Some(step_n) = step_num
                    {
                        let sim = &s.simulations[simulation_index];
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
                                "species" =>
                                {
                                    Get: Endpoint::Simulation( Simulation::SpeciesInEpoch  { s: ss.clone() } ),
                                    ":species" => Get: Endpoint::Simulation( Simulation::SpeciesInEpochOne { s: ss.clone() }),
                                },
                                "environment" =>
                                {
                                    Get: Endpoint::Simulation( Simulation::EnvironmentInEpoch { s: ss.clone() } ),
                                }
                            }
                        },
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
