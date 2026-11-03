use kdiff4_protocol::engine::ComparisonEngine;

use crate::builtin_engine::BuiltinEngine;


/// The comparison engines the host can use. The built-in engine is always present and is the default.
pub struct EngineRegistry {
    engines: Vec<Box<dyn ComparisonEngine>>,
}


impl Default for EngineRegistry {
    fn default() -> EngineRegistry {
        EngineRegistry { engines: vec![Box::new(BuiltinEngine)] }
    }
}


impl EngineRegistry {
    /// Returns false, and keeps the registered engine, when an engine with the same id is already registered.
    pub fn register(&mut self, engine: Box<dyn ComparisonEngine>) -> bool {
        let id = engine.capabilities().id;
        if self.get(&id).is_some() {
            log::warn!("Comparison engine {id} is already registered");
            return false;
        }
        log::info!("Registered comparison engine {id}");
        self.engines.push(engine);
        true
    }


    pub fn get(&self, id: &str) -> Option<&dyn ComparisonEngine> {
        self.engines
            .iter()
            .find(|engine| engine.capabilities().id == id)
            .map(|engine| engine.as_ref())
    }


    pub fn default_engine(&self) -> &dyn ComparisonEngine {
        self.engines[0].as_ref()
    }


    pub fn ids(&self) -> Vec<String> {
        self.engines
            .iter()
            .map(|engine| engine.capabilities().id)
            .collect()
    }
}


#[cfg(test)]
mod tests {
    use kdiff4_protocol::engine::{CharDiff, CompareInput, Comparison, ComparisonOptions, EngineCapabilities, EngineError, Job, RegionStatus, SourceLines};

    use super::*;
    use crate::builtin_engine::BUILTIN_ENGINE_ID;


    struct NamedEngine(&'static str);


    impl ComparisonEngine for NamedEngine {
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities { id: self.0.to_string(), ..BuiltinEngine.capabilities() }
        }


        fn compare(&self, _input: &CompareInput<'_>, _job: &Job<'_>) -> Result<Comparison, EngineError> {
            Err(EngineError::Failed("not a real engine".to_string()))
        }


        fn classify(&self, _hunks: &[SourceLines<'_>]) -> Result<RegionStatus, EngineError> {
            Err(EngineError::Failed("not a real engine".to_string()))
        }


        fn char_diff(&self, _row: &[Option<&str>], _options: &ComparisonOptions) -> Result<CharDiff, EngineError> {
            Err(EngineError::Failed("not a real engine".to_string()))
        }
    }


    #[test]
    fn built_in_engine_is_the_default() {
        let registry = EngineRegistry::default();
        assert_eq!(registry.default_engine().capabilities().id, BUILTIN_ENGINE_ID);
        assert_eq!(registry.ids(), [BUILTIN_ENGINE_ID]);
    }


    #[test]
    fn registers_engines_by_id() {
        let mut registry = EngineRegistry::default();
        assert!(registry.register(Box::new(NamedEngine("other"))));
        assert_eq!(registry.ids(), [BUILTIN_ENGINE_ID, "other"]);
        assert!(registry.get("other").is_some());
        assert!(registry.get("missing").is_none());
        assert_eq!(registry.default_engine().capabilities().id, BUILTIN_ENGINE_ID);
    }


    #[test]
    fn rejects_duplicate_ids() {
        let mut registry = EngineRegistry::default();
        assert!(!registry.register(Box::new(NamedEngine(BUILTIN_ENGINE_ID))));
        assert!(registry.register(Box::new(NamedEngine("other"))));
        assert!(!registry.register(Box::new(NamedEngine("other"))));
        assert_eq!(registry.ids(), [BUILTIN_ENGINE_ID, "other"]);
    }
}
