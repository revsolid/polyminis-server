use lru_cache::LruCache;
use polyminis_core::serialization::*;
use polyminis_core::simulation::*;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use std::thread;

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

                // Simulation Data is:
                //     Simulation Configuration
                //     Epoch Data
                //     Species Data
                //     //TODO: Right now Epoch Data and Simulation Configuration are a bit coupled

                let mut sim = Simulation::new_from_json(&simulation_data).unwrap(); 
                let record_for_database = true;
                let record_for_simulation = true;

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

                trace!("Starting Simulation Loop");
                'epoch: loop
                {
                    let break_loop = sim.step();


                    if record_for_simulation
                    {
                        trace!("Recording Data for Step");
                        let step_json = sim.get_epoch().serialize(&mut SerializationCtx::new_from_flags(PolyminiSerializationFlags::PM_SF_DYNAMIC));
                        {
                            let mut w = workspace.write().unwrap();
                            w.epochs.get_mut(&(sim.epoch_num as u32)).unwrap().steps.push(step_json);
                        }
                    }

                    if break_loop
                    {
                        trace!("Ending Simulation");
                        break 'epoch;
                    }
                }

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
