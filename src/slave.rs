use lru_cache::LruCache;

use polyminis_core::environment::*;
use polyminis_core::evaluation::*;
use polyminis_core::genetics::*;
use polyminis_core::physics::*;
use polyminis_core::serialization::*;
use polyminis_core::simulation::*;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use std::thread;

use env_logger;

#[derive(Clone)]
pub struct EpochState
{
    epoch_num: usize,
    evaluated: bool,
    pub steps: pmJsonArray,
    pub species: pmJsonArray,
    pub environment: pmJsonObject,
    pub evaluation_data: pmJsonObject,
    pub persistable_data: pmJsonObject,
    pub persistable_species_data: pmJsonObject,
}
impl EpochState
{
    pub fn new(i: usize) -> EpochState
    {
        EpochState { epoch_num: i, steps: pmJsonArray::new(),  species: pmJsonArray::new(), evaluated: false, evaluation_data: pmJsonObject::new(),
                     environment: pmJsonObject::new(), persistable_data: pmJsonObject::new(), persistable_species_data: pmJsonObject::new() }
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

pub enum WorkerThreadActions
{
    Simulate { simulation_data: Json },
    Advance  { simulation_data: Json },
    Exit,
}
impl WorkerThreadActions
{
    
    pub fn handle(&self, workspace: &mut Arc<RwLock<WorkerThreadState>>)
    {
        match *self
        {
            WorkerThreadActions::Simulate { simulation_data: ref simulation_data } =>
            {
                // Avoid loading anything already in memory
                match simulation_data
                {
                    &Json::Object(ref json_obj) =>
                    {
                        let mut w = workspace.write().unwrap();
                        let epoch_num = json_obj.get("EpochNum").unwrap_or(&Json::U64(0)).as_u64().unwrap_or(0);
                        match w.epochs.get_mut(& (epoch_num as u32))
                        {
                            Some(ref epoch_state) =>
                            {
                                // Early break
                                if (epoch_state.evaluated)
                                {
                                    trace!("Simulation Epoch Num: {} already simulated...returning", epoch_num);
                                    return
                                }
                            }
                            _ => 
                            {}
                        }
                    },
                    _ =>
                    {
                    }
                }

                // Simulation Data is:
                //     Simulation Configuration
                //     Epoch Data
                //     Species Data
                //     //TODO: Right now Epoch Data and Simulation Configuration are a bit coupled
                //
                let mut sim = Simulation::new_from_json(&simulation_data).unwrap(); 

                trace!(
                "Simulation Loaded: {}",
                sim.get_epoch().get_species().len());



                let record_for_database = true;
                let record_for_playback = true;

                trace!("Recording Static Data");
                {
                    let mut epoch_state = EpochState::new(sim.epoch_num);
                    let mut ctx = SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_STATIC);

                    for s in sim.get_epoch().get_species() 
                    {
                        epoch_state.species.push(s.serialize(&mut ctx));
                    }
                    
                    let mut env_obj = pmJsonObject::new();
                    env_obj.insert("Environment".to_owned(), sim.get_epoch().get_environment().serialize(&mut ctx));
                    epoch_state.environment = env_obj;

                    let mut w = workspace.write().unwrap();
                    w.epochs.insert(sim.epoch_num as u32, epoch_state);
                }

                let sim_type = match simulation_data
                {
                    &Json::Object(ref json_obj) =>
                    {
                        match json_obj.get("SimulationType")
                        {
                            None =>
                            {
                                "Legacy Style".to_owned()
                            },
                            Some(sim_type) =>
                            {
                                sim_type.as_string().unwrap().to_owned()
                            }
                        }
                    },
                    _ =>
                    {
                        "Legacy Style".to_owned()
                    }
                };

                self.SimulationEpochLoop(&mut sim, sim_type, workspace, record_for_playback, record_for_database, simulation_data); 
                // SimulationEpochLoop (..., simulation_data,...);

                trace!("Evaluating Species");
                sim.get_epoch_mut().evaluate_species(); 
                {
                    let mut w = workspace.write().unwrap();
                    let mut_epoch = w.epochs.get_mut(&(sim.epoch_num as u32)).unwrap();
                    mut_epoch.evaluated = true;

                    trace!("Recording Evaluation Data");
                    for s in sim.get_epoch().get_species()
                    {
                        mut_epoch.evaluation_data.insert(s.get_name().clone(), s.get_best().serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_STATS)));
                    }

                    if record_for_database
                    {
                        trace!("Recording Persistable Data");
                        mut_epoch.persistable_data  = match sim.get_epoch().serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DB))
                        {
                            Json::Object(data) =>
                            {
                                data
                            },
                            _ => { pmJsonObject::new() }
                        };


                        for s in sim.get_epoch().get_species()
                        {
                            mut_epoch.persistable_species_data.insert(s.get_name().clone(), s.serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DB)));
                        }
                    }
                }
            },
            WorkerThreadActions::Advance{ simulation_data: ref simulation_data } =>
            {
                let mut sim_opt = Simulation::new_from_json(&simulation_data); 

                let mut sim = Simulation::new_from_json(&simulation_data).unwrap(); 
                // Fil the simulation up with data
                sim.advance_epoch();
                {
                    let mut epoch_state = EpochState::new(sim.epoch_num);
                    epoch_state.persistable_data  = match sim.get_epoch().serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DB))
                    {
                        Json::Object(data) =>
                        {
                            data
                        },
                        _ => { pmJsonObject::new() }
                    };

                    for s in sim.get_epoch().get_species()
                    {
                        epoch_state.persistable_species_data.insert(s.get_name().clone(), s.serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DB)));
                    }
                    let mut w = workspace.write().unwrap();
                    w.epochs.insert(sim.epoch_num as u32, epoch_state);
                }
            },
            _ =>
            {
            }
        }
    }

    //fn SimulationEpochLoop(&self, sim: &mut Simulation, sim_type: String, workspace: &mut Arc<RwLock<WorkerThreadState>>, record_for_playback: bool, record_for_database: bool)
    fn SimulationEpochLoop(&self, sim: &mut Simulation, sim_type: String, workspace: &mut Arc<RwLock<WorkerThreadState>>, record_for_playback: bool, record_for_database: bool,
                           sim_data: &Json)
    {
        //let sim_type = "Legacy Style";
        trace!("Starting Simulation Epoch Loop - Type {}", sim_type); //TODO: Trace params
        match sim_type.as_ref()
        {  
            // CreatureObservation case: - Select the best (or best few) of each species keep
            // running indefinetiley (?)
            //
            // CreatureDesign case: - Start with a specific Seed and then run the batery of 
            // scenarios using the provided translation table
            //
            // Chronos case - Run the species through a gauntlet of scenarios (Solo Run or 'Legacy'
            // style)
            // 

            // Creature Design
            "Creature Design" =>
            {
                // This is basically a Solo Run ?
            },
            
            // Solo Run
            "Solo Run" =>
            {
                // TODO: For now we'll be using the same setup for all runs,
                // this should be parameterizable
                
                let scenarios: Vec<(Environment, PGAConfig, Box<PlacementFunction>)> =
                    match sim_data.as_object().unwrap().get("Scenarios")
                    {
                        Some(&Json::Array(ref scenarios_json)) =>
                        {
                            let ser_ctx = &mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DB);
                            scenarios_json.iter().map(|ref scenario|
                            {
                                let scenario_object = scenario.as_object().unwrap();
                                let env = Environment::new_from_json(scenario_object.get("Environment").unwrap()).unwrap();
                                let pgaconfig = PGAConfig::new_from_json(scenario_object.get("GAConfiguration").unwrap(), ser_ctx).unwrap();

                                let low_x = scenario_object.get("StartingLowX").unwrap_or(&Json::Null).as_f64().unwrap_or(0.0) as f32;
                                let low_y = scenario_object.get("StartingLowY").unwrap_or(&Json::Null).as_f64().unwrap_or(0.0) as f32;
                                let high_x = scenario_object.get("StartingHighX").unwrap_or(&Json::Null).as_f64().unwrap_or(env.dimensions.0 as f64) as f32;
                                let high_y = scenario_object.get("StartingHighY").unwrap_or(&Json::Null).as_f64().unwrap_or(env.dimensions.1 as f64) as f32;
                                let placement_func: Box<PlacementFunction> = Box::new( move | ctx: &mut PolyminiRandomCtx |
                                         {
                                           ( (ctx.gen_range(low_x, high_x) as f32).floor(),
                                             (ctx.gen_range(low_y, high_y) as f32).floor())
                                         });
                                (env,
                                 pgaconfig,
                                 placement_func)
                            }).collect()
                        },
                        _ =>
                        {
                            let mut new_env = sim.get_epoch().get_environment().clone();
                            
                            let evaluators = vec![ FitnessEvaluator::OverallMovement { weight: 0.5 },
                                                   FitnessEvaluator::DistanceTravelled { weight: 0.5 },
                                                   FitnessEvaluator::Alive { weight: 15.0 },
                                                   FitnessEvaluator::Shape { weight: 8.0 },
                                                   FitnessEvaluator::PositionsVisited { weight: 3.5 },
                                                 ];

                            let cfg = PGAConfig { population_size: 50,
                                                  percentage_elitism: 0.20, percentage_mutation: 0.3, fitness_evaluators: evaluators,
                                                  genome_size: 4, accumulates_over: true };

                            vec![
                                (new_env.clone(), cfg.clone(),
                                 Box::new( | ctx: &mut PolyminiRandomCtx |
                                 {
                                   ( (ctx.gen_range(12.0, 18.0) as f32).floor(),
                                     (ctx.gen_range(12.0, 18.0) as f32).floor())
                                 })),
                                (new_env.clone(), cfg.clone(),
                                 Box::new( | ctx: &mut PolyminiRandomCtx |
                                 {
                                   ( (ctx.gen_range(32.0, 38.0) as f32).floor(),
                                     (ctx.gen_range(32.0, 38.0) as f32).floor())
                                 })),
                                 (new_env.clone(), cfg.clone(),
                                 Box::new( | ctx: &mut PolyminiRandomCtx |
                                 {
                                   ( (ctx.gen_range(12.0, 18.0) as f32).floor(),
                                     (ctx.gen_range(32.0, 38.0) as f32).floor())
                                 })),
                                 (new_env.clone(), cfg.clone(),
                                 Box::new( | ctx: &mut PolyminiRandomCtx |
                                 {
                                   ( (ctx.gen_range(32.0, 38.0) as f32).floor(),
                                     (ctx.gen_range(12.0, 18.0) as f32).floor())
                                 })),
                             ]    
                        }
                    };


                trace!("Solo Run includes {} scenarios", scenarios.len());
                sim.get_epoch_mut().solo_run(&scenarios);


                trace!("Done solo running");
            }, 

            // Creature Observation
            "Creature Observation" =>
            {
                // This is basically 'legacy' style 
            },

            // Legacy Sytle 
            "Legacy Style" =>  
            {
                'legacy_style: loop
                {
                    let break_loop = sim.step();


                    if record_for_playback
                    {
                        let step_json = sim.get_epoch().serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DYNAMIC));
                        {
                            let mut w = workspace.write().unwrap();
                            w.epochs.get_mut(&(sim.epoch_num as u32)).unwrap().steps.push(step_json);
                        }
                    }

                    if break_loop
                    {
                        trace!("Ending Simulation");
                        break 'legacy_style;
                    }
                }
            },
            &_ =>
            {
                trace!("Pretty fucky wtf");
            }
        }
    }
}

pub struct WorkerThreadState
{
    // Control and Metadata
    pub actions: VecDeque<WorkerThreadActions>,
    pub thread_hndl: Option<thread::Thread>,

    // Data 
    pub epochs: LruCache<u32, EpochState>,
}
impl WorkerThreadState
{
    pub fn new() -> WorkerThreadState
    {
        WorkerThreadState { actions: VecDeque::new(), thread_hndl: None, epochs: LruCache::new(10) }
    }

    pub fn worker_thread_main(mut workspace: Arc<RwLock<WorkerThreadState>>)
    {
        let _ = env_logger::init();
        {
            let mut w = workspace.write().unwrap();
            w.thread_hndl = Some(thread::current())
        }

        //
        'main: loop
        {
            let mut action = None;
            {
                let mut w = workspace.write().unwrap();
                action = w.actions.pop_front();
            }

            if action.is_none()
            {
                thread::park();
                {
                    let mut w = workspace.write().unwrap();
                    action = w.actions.pop_front();
                }
            }

            let action_enum = action.unwrap();
            action_enum.handle(&mut workspace);
            match action_enum
            {
                WorkerThreadActions::Exit =>
                {
                    break 'main;
                },
                _ =>
                {
                    thread::park();
                }
            }
        }

        {
            // Cleanup ?
        }
    }
}
