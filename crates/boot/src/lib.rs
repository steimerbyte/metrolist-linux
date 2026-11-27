//! Boot registry and timeline for the start-up sequence.
//! Modules are announced by `crates/<dir>/module.meta`; build.rs turns them into `MODULES`.

pub mod gen;
mod timeline;

pub use gen::Meta;
pub use timeline::{state_at, total, Phase, State, HANDOVER, LOAD, SLOT};

#[derive(Clone, Copy, Debug)]
pub struct Module {
    pub name: &'static str,
    pub label: &'static str,
    pub order: u16,
    pub init: Option<fn() -> bool>,
}

pub const MODULES: &[Module] = &include!(concat!(env!("OUT_DIR"), "/modules.rs"));

/// Runs a module's optional init hook. Hooks must be cheap and non-blocking; the
/// boot overlay never waits for them. `None` means the module has no hook.
pub fn run_init(module: &Module) -> Option<bool> {
    module.init.map(|init| init())
}
