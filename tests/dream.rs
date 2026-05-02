use tempfile::TempDir;

mod common;
use common::write_memory_yaml_with_word_count;

#[test]
fn dream_guard_integration() {
    let dir = TempDir::new().unwrap();
    let plan = dir.path();

    assert!(!ravel_lite::dream::should_dream(plan, 1500));

    write_memory_yaml_with_word_count(plan, 100);
    ravel_lite::dream::update_dream_word_count(plan);

    write_memory_yaml_with_word_count(plan, 200);
    assert!(!ravel_lite::dream::should_dream(plan, 1500));

    write_memory_yaml_with_word_count(plan, 2000);
    assert!(ravel_lite::dream::should_dream(plan, 1500));

    ravel_lite::dream::update_dream_word_count(plan);
    assert!(!ravel_lite::dream::should_dream(plan, 1500));
}
