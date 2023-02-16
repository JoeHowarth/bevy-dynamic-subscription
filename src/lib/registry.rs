use bevy::reflect::{erased_serde, GetTypeRegistration};
use bevy::{
    ecs::component::ComponentId,
    prelude::*,
    reflect::{ReflectFromPtr, TypeRegistry},
    utils::HashMap,
};
use bevy_ecs_dynamic::dynamic_query::{self, DynamicQuery, FetchKind, FetchResult, FilterKind};
use serde::{Deserialize, Serialize};
use serde_json;
use serde_json::value::RawValue;
use std::sync::RwLock;
use std::{any::TypeId, io};

pub type ShortName = String;

#[derive(Default, Resource)]
pub struct ComponentIdRegistry {
    short_names: HashMap<ShortName, ComponentId>,
    type_ids: HashMap<TypeId, ComponentId>,
}

impl ComponentIdRegistry {
    pub fn register<T: Component>(
        &mut self,
        component_id: ComponentId,
        short_name: impl Into<ShortName>,
    ) {
        self.short_names.insert(short_name.into(), component_id);
        self.type_ids
            .insert(std::any::TypeId::of::<T>(), component_id);
    }

    pub fn short_name(&self, short_name: impl AsRef<str>) -> ComponentId {
        self.short_names.get(short_name.as_ref()).unwrap().clone()
    }
}

pub trait RegistryExt {
    fn register<T: Component + GetTypeRegistration>(&mut self);
}

impl RegistryExt for World {
    fn register<T: Component + GetTypeRegistration>(&mut self) {
        use bevy::prelude::*;
        let component_id = self.init_component::<T>();
        let short_name = {
            let type_registry = self.get_resource_or_insert_with(AppTypeRegistry::default);
            let mut type_registry = type_registry.write();
            type_registry.register::<T>();
            let short_name = type_registry
                .get(std::any::TypeId::of::<T>())
                .unwrap()
                .short_name();
            short_name.to_string()
        };
        let mut registry = self.get_resource_or_insert_with(ComponentIdRegistry::default);
        registry.register::<T>(component_id, short_name);
    }
}

impl RegistryExt for App {
    fn register<T: Component + GetTypeRegistration>(&mut self) {
        self.world.register::<T>()
    }
}
