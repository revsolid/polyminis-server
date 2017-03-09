use lru_cache::LruCache;
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
                        if w.epochs.get_mut(& (epoch_num as u32)).is_some()
                        {
                            // Early break
                            return
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
                                sim_type.to_string()
                            }
                        }
                    },
                    _ =>
                    {
                        "Legacy Style".to_owned()
                    }
                };

                self.SimulationEpochLoop(&mut sim, sim_type, workspace, record_for_playback, record_for_database); 

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
    fn SimulationEpochLoop(&self, sim: &mut Simulation, sim_type: String, workspace: &mut Arc<RwLock<WorkerThreadState>>, record_for_playback: bool, record_for_database: bool)
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
                // TODO: For now is ok to send an empty vec
                sim.get_epoch_mut().solo_run(&vec![]);
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
