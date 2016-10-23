extern crate polyminis_core;

#[macro_use]
extern crate rustful;

use std::error::Error;

use rustful::{Context, Handler, Response, Server, TreeRouter};

use polyminis_core::control::*;
use polyminis_core::environment::*;
use polyminis_core::morphology::*;
use polyminis_core::polymini::*;
use polyminis_core::serialization::*;
use polyminis_core::species::*;

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



mod State
{
    use std::sync::{Arc, RwLock};
    use std::{thread, time};
    use polyminis_core::control::*;
    use polyminis_core::environment::*;
    use polyminis_core::morphology::*;
    use polyminis_core::polymini::*;
    use polyminis_core::serialization::*;
    use polyminis_core::species::*;

    struct WorkerThreadState
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
            let mut sim = Simulation::new();

            // Critical Section
            {
                let mut w = workspace.write().unwrap();
                w.static_state = sim.serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_STATIC)).to_string();
            }
            
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
            }
        }
    }

    struct SimulationState
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
                    //  TODO: Go and get static data from the Simulation
                    let w = self.work_thread_state.read().unwrap();
                    self.static_state = Some(w.static_state.clone());
                }
            }
            &self.static_state
        }
    }

    struct ServerState
    {
        simulations: Vec<SimulationState>,
    }

    impl ServerState
    {
        pub fn add_simulation(&mut self/* Data ? */ )
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
    }
}


mod EndpointHandlers 
{
    enum Simulation
    {}

    enum Management
    {}
}




fn main()
{
    let mut shared_space = Arc::new(RwLock::new(ServiceMemory { simulations: vec![ SimulationMemory { steps: vec![] }]}));


    let thread_copy = shared_space.clone();
    thread::spawn(move || {
        let chromosomes = vec![[0, 0x09, 0x6A, 0xAD],
                               [0, 0x0B, 0xBE, 0xDA],
                               [0,    0, 0xBE, 0xEF],
                               [0,    0, 0xDB, 0xAD]];

        let p1 = Polymini::new(Morphology::new(chromosomes, &TranslationTable::new()),
                               Control::new());
        let mut s = Simulation::new();
        s.add_species(Species::new(vec![p1]));
        s.add_object((10.0, 2.0), (1, 1));
        for _ in 0..10
        {
            s.step();
            let step_string = format!("{}", s.serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DYNAMIC)));

            // Critical Section
            {
                let mut service_mem = thread_copy.write().unwrap();
                service_mem.simulations[0].steps.push(step_string);
            }

            let five_s = time::Duration::from_millis(5000);
            thread::sleep(five_s); 
        }

    });

    //Build and run the server.
    let server_result = Server {
        //Turn a port number into an IPV4 host address (0.0.0.0:8080 in this case).
        host: 8080.into(),

        //Create a TreeRouter and fill it with handlers.
        handlers: insert_routes!
        {
            TreeRouter::new() =>
            {
                //Root
                Get: Api::TestEmpty,

                //Simulation API
                "simulation" => Get: Api::TestSim
                {
                    service_memory: shared_space.clone()
                },

                "simulation/:step" => Get: Api::TestSim
                {
                    service_memory: shared_space.clone()
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
            
            Api::TestSim { ref service_memory } =>
            {
                let ro_mem = service_memory.read().unwrap();
                if ro_mem.simulations.len() >  0
                {
                    if let Some(step) = context.variables.get("step")
                    {
                        let step_inx_result = usize::from_str_radix(&step, 10);
                        match step_inx_result
                        {
                            Ok(s_inx) =>
                            {
                                if ro_mem.simulations[0].steps.len() < s_inx
                                {
                                    response.send(format!("Step {} not ready", s_inx))
                                }
                                else
                                {
                                    let step_string = ro_mem.simulations[0].steps[s_inx - 1].clone();
                                    response.send(step_string)
                                }
                            },
                            Err(e) =>
                            {
                                response.send(format!("{:?}", e))
                            }
                        }
                    }
                    else
                    {
                        let mut simulation_dump: String = "".to_string();
                        for ss in &ro_mem.simulations[0].steps
                        {
                            simulation_dump = simulation_dump + ss;
                        }
                        response.send(simulation_dump);
                    }
                }
                else
                {
                    response.send("No Simulation Data available")
                }
            },
            Api::TestEmpty =>
            {
                response.send("fuck you")
            },
        };
    }
}
