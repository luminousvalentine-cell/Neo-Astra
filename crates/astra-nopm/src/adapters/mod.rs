pub mod cargo;
pub mod git;
pub mod npm;
pub mod nuget;
pub mod oci;
pub mod pypi;
pub mod registry;

use crate::package_id::DependencyRequest;
use crate::resolve::Descriptor;

#[derive(Clone, Debug)]
pub struct AdapterResolution {
    pub descriptor: Descriptor,
    pub request: DependencyRequest,
}

pub fn insert_dependency(dependencies: &mut Vec<(String, String)>, name: String, spec: String) {
    if let Some(existing) = dependencies.iter_mut().find(|(key, _)| key == &name) {
        existing.1 = spec;
    } else {
        dependencies.push((name, spec));
    }
}
