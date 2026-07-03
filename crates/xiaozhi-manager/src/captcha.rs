use std::collections::HashMap;
use std::sync::Mutex;

use rand::Rng;

struct Challenge {
    answer: i32,
    expires_at: chrono::DateTime<chrono::Utc>,
}

pub struct CaptchaStore {
    inner: Mutex<HashMap<String, Challenge>>,
}

impl CaptchaStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({ "enabled": true })
    }

    pub fn new_challenge(&self) -> (String, String) {
        let mut rng = rand::thread_rng();
        let a: i32 = rng.gen_range(1..10);
        let b: i32 = rng.gen_range(1..10);
        let id = uuid::Uuid::new_v4().to_string();
        let prompt = format!("{a} + {b} = ?");
        let answer = a + b;
        let expires = chrono::Utc::now() + chrono::Duration::minutes(5);
        self.inner.lock().unwrap().insert(
            id.clone(),
            Challenge {
                answer,
                expires_at: expires,
            },
        );
        (id, prompt)
    }

    pub fn verify(&self, id: &str, answer: &str) -> bool {
        let mut guard = self.inner.lock().unwrap();
        let now = chrono::Utc::now();
        guard.retain(|_, v| v.expires_at > now);
        let Some(ch) = guard.remove(id) else {
            return false;
        };
        answer.trim().parse::<i32>().ok() == Some(ch.answer)
    }
}
