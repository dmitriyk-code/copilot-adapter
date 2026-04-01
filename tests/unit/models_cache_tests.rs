use std::time::Duration;

use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::copilot::types::{Model, ModelList};

/// Helper to create a sample `ModelList` for tests.
fn sample_model_list() -> ModelList {
    ModelList {
        object: "list".to_string(),
        data: vec![
            Model {
                id: "gpt-4o".to_string(),
                object: "model".to_string(),
                created: 1700000000,
                owned_by: "openai".to_string(),
            },
            Model {
                id: "claude-sonnet-4".to_string(),
                object: "model".to_string(),
                created: 1700000001,
                owned_by: "anthropic".to_string(),
            },
        ],
    }
}

#[tokio::test]
async fn get_returns_none_on_empty_cache() {
    let cache = ModelsCache::new(Duration::from_secs(300));
    assert!(cache.get().await.is_none());
}

#[tokio::test]
async fn set_then_get_returns_cached_data() {
    let cache = ModelsCache::new(Duration::from_secs(300));
    let models = sample_model_list();

    cache.set(models.clone()).await;
    let cached = cache.get().await;

    assert!(cached.is_some());
    let cached = cached.unwrap();
    assert_eq!(cached.object, "list");
    assert_eq!(cached.data.len(), 2);
    assert_eq!(cached.data[0].id, "gpt-4o");
    assert_eq!(cached.data[1].id, "claude-sonnet-4");
}

#[tokio::test]
async fn get_returns_none_after_ttl_expires() {
    let cache = ModelsCache::new(Duration::from_millis(50));
    cache.set(sample_model_list()).await;

    // Value should be present immediately
    assert!(cache.get().await.is_some());

    // Wait for TTL to expire
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(cache.get().await.is_none());
}

#[tokio::test]
async fn invalidate_clears_cache() {
    let cache = ModelsCache::new(Duration::from_secs(300));
    cache.set(sample_model_list()).await;
    assert!(cache.get().await.is_some());

    cache.invalidate().await;
    assert!(cache.get().await.is_none());
}

#[tokio::test]
async fn concurrent_reads_dont_block() {
    use std::sync::Arc;

    let cache = Arc::new(ModelsCache::new(Duration::from_secs(300)));
    cache.set(sample_model_list()).await;

    let mut handles = Vec::new();
    for _ in 0..10 {
        let cache = Arc::clone(&cache);
        handles.push(tokio::spawn(async move { cache.get().await }));
    }

    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().data.len(), 2);
    }
}

#[tokio::test]
async fn set_overwrites_previous_value() {
    let cache = ModelsCache::new(Duration::from_secs(300));
    cache.set(sample_model_list()).await;

    let new_models = ModelList {
        object: "list".to_string(),
        data: vec![Model {
            id: "gpt-5".to_string(),
            object: "model".to_string(),
            created: 1700000002,
            owned_by: "openai".to_string(),
        }],
    };
    cache.set(new_models).await;

    let cached = cache.get().await.unwrap();
    assert_eq!(cached.data.len(), 1);
    assert_eq!(cached.data[0].id, "gpt-5");
}
