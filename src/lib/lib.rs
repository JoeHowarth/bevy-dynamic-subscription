#![allow(unused, dead_code)]

pub mod registry;

use self::registry::ComponentIdRegistry;
use bevy::reflect::{erased_serde, GetTypeRegistration};
use bevy::{
    ecs::component::ComponentId,
    prelude::*,
    reflect::{ReflectFromPtr, TypeRegistry},
    utils::HashMap,
};
use bevy_ecs_dynamic::dynamic_query::{self, DynamicQuery, FetchKind, FetchResult, FilterKind};
pub use registry::{RegistryExt, ShortName};
use serde::{Deserialize, Serialize};
use serde_json;
use serde_json::value::RawValue;
use std::sync::RwLock;
use std::{any::TypeId, io};

fn main() {
    println!("Hello, world!");
}

#[derive(Default, Resource)]
pub struct EcsSubApi {
    pub queries: Box<RwLock<HashMap<QueryId, (QuerySubReq, DynamicQuery)>>>,
}

impl EcsSubApi {
    // pub fn system(api: ResMut<EcsSubApi>, world: &World, )

    pub fn subscribe_resource(&self, res: ResourceSubReq) {
        todo!()
    }
    pub fn subscribe_components(&self, query: QuerySubReq, world: &World) {
        let registry = world.get_resource::<ComponentIdRegistry>().unwrap();
        let component_fetches = query
            .fetch
            .iter()
            .map(|short_name| FetchKind::Ref(registry.short_name(short_name)))
            .collect();
        let filters = query
            .filter
            .iter()
            .map(|filter| filter.resolve_components(registry))
            .collect();

        let dyn_query = DynamicQuery::new(world, component_fetches, filters).unwrap();
        self.queries
            .write()
            .unwrap()
            .insert(query.id.clone(), (query, dyn_query));
    }

    pub fn run_all_queries(&self, world: &World) -> Vec<QuerySubResp> {
        let mut queries = self.queries.write().unwrap();
        queries
            .iter_mut()
            .map(|(_id, (query, dyn_query))| self.run_query_internal(world, &query, dyn_query))
            .collect()
    }

    pub fn run_query(&self, world: &World, id: &QueryId) -> QuerySubResp {
        let mut queries = self.queries.write().unwrap();
        let (query, dyn_query) = queries.get_mut(id).unwrap();
        self.run_query_internal(world, query, dyn_query)
    }

    pub fn run_query_internal(
        &self,
        world: &World,
        query: &QuerySubReq,
        dyn_query: &mut DynamicQuery,
    ) -> QuerySubResp {
        let type_registry = &*world.get_resource::<AppTypeRegistry>().unwrap().read();
        let matches = dyn_query
            .iter(world)
            .map(|raw| {
                let components = raw
                    .items
                    .iter()
                    .zip(query.fetch.iter())
                    .map(|(fetch_res, short_name)| {
                        let FetchResult::Ref(ptr) = fetch_res else {
                            unimplemented!();
                        };
                        let type_id = type_registry
                            .get_with_short_name(short_name)
                            .unwrap()
                            .type_id();
                        let reflect = type_registry
                            .get_type_data::<ReflectFromPtr>(type_id)
                            .unwrap();

                        // SAFETY:
                        // `val` is a pointer to value of the type that the `ReflectFromPtr` was constructed for,
                        // because the mapping from `ComponentId -> TypeId` is immutable and `ReflectFromPtr` is checked to be
                        // for the type of the `WorldBase`'s type id.
                        let reflect = unsafe { reflect.as_reflect_ptr(*ptr) };
                        let value = type_registry
                            .get_type_data::<ReflectSerialize>(type_id)
                            .unwrap()
                            .get_serializable(reflect);
                        let serialized = RawValue::from_string(value.borrow().to_json()).unwrap();
                        (short_name.clone(), serialized)
                    })
                    .collect();
                (raw.entity.to_bits(), components)
            })
            .collect();
        QuerySubResp { matches }
    }
}

pub trait ToJson {
    fn to_json(&self) -> String;
}

impl<T: erased_serde::Serialize> ToJson for T {
    fn to_json(&self) -> String {
        let mut v = Vec::<u8>::with_capacity(100);
        let json = &mut serde_json::Serializer::new(&mut v);
        let mut json = <dyn erased_serde::Serializer>::erase(json);
        self.erased_serialize(&mut json);
        unsafe { String::from_utf8_unchecked(v) }
    }
}

pub type JsonString = String;
pub type QueryId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShortNameFilter {
    With(ShortName),
    Without(ShortName),
    Changed(ShortName),
    // Added(ShortName),
}

impl ShortNameFilter {
    pub fn resolve_components(&self, registry: &ComponentIdRegistry) -> FilterKind {
        match self {
            ShortNameFilter::With(s) => FilterKind::With(registry.short_name(s)),
            ShortNameFilter::Without(s) => FilterKind::Without(registry.short_name(s)),
            ShortNameFilter::Changed(s) => FilterKind::Changed(registry.short_name(s)),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ResourceSubReq {
    pub short_name: ShortName,
    pub only_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuerySubReq {
    pub id: QueryId,
    pub fetch: Vec<ShortName>,
    pub filter: Vec<ShortNameFilter>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QuerySubResp {
    pub matches: Vec<(u64, HashMap<ShortName, Box<RawValue>>)>,
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Duration;

    use bevy::{app::ScheduleRunnerSettings, prelude::*};

    #[derive(Debug, Component, Reflect, serde::Serialize)]
    #[reflect(Serialize)]
    struct Location {
        pub city: String,
    }

    #[derive(Debug, Component, Reflect, serde::Serialize)]
    #[reflect(Serialize)]
    struct Health {
        pub health: u32,
    }

    #[test]
    fn test() {
        App::new()
            .insert_resource(ScheduleRunnerSettings::run_once())
            .add_plugins(MinimalPlugins)
            .add_startup_system(setup)
            .add_system(run)
            .run();
    }

    fn setup(world: &mut World) {
        world.register::<Health>();
        world.register::<Location>();

        world.spawn((
            Location {
                city: "Salmon".into(),
            },
            Health { health: 99 },
        ));
        world.spawn((Location { city: "NYC".into() }, Health { health: 50 }));
        world.spawn((Location { city: "SLC".into() },));
        world.spawn(Health { health: 40 });

        let api = EcsSubApi::default();
        api.subscribe_components(
            QuerySubReq {
                id: "Both".into(),
                fetch: vec!["Location".into()],
                filter: vec![],
            },
            &world,
        );
        world.insert_resource(api);
    }

    fn run(world: &World) {
        let api = world.get_resource::<EcsSubApi>().unwrap();
        let resp = api.run_query(&world, &"Both".to_string());
        let resp_json = serde_json::to_string(&resp).unwrap();

        let raw_value = serde_json::value::to_raw_value(&Location { city: "NYC".into() }).unwrap();
        let expected = QuerySubResp {
            matches: vec![
                (
                    0,
                    HashMap::from_iter([(
                        "Location".to_string(),
                        serde_json::value::to_raw_value(&Location {
                            city: "Salmon".into(),
                        })
                        .unwrap(),
                    )]),
                ),
                (
                    1,
                    HashMap::from_iter([(
                        "Location".to_string(),
                        serde_json::value::to_raw_value(&Location { city: "NYC".into() }).unwrap(),
                    )]),
                ),
                (
                    2,
                    HashMap::from_iter([(
                        "Location".to_string(),
                        serde_json::value::to_raw_value(&Location { city: "SLC".into() }).unwrap(),
                    )]),
                ),
            ],
        };
        let expected_json = serde_json::to_string(&expected).unwrap();
        assert_eq!(resp_json, expected_json);
    }
}
