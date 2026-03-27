use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct Cache {
    data: Arc<Mutex<HashMap<String, String>>>,
}

impl Cache {
    pub fn new() -> Self {
        Cache {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set(&self, key: &str, value: &str) {
        let mut cache = self.data.lock().unwrap();
        cache.insert(key.to_string(), value.to_string());
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let cache = self.data.lock().unwrap();
        cache.get(key).cloned()
    }
}
