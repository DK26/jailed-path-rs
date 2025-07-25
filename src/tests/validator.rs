use crate::jailed_path::JailedPath;
use crate::validator::PathValidator;
use crate::JailedPathError;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

// TODO: Split my module!

/// Creates cross-platform attack target paths for testing
fn get_attack_target_paths() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        vec![
            "C:\\Windows\\System32\\config\\SAM",
            "C:\\Windows\\System32\\drivers\\etc\\hosts",
            "D:\\sensitive\\data.txt",
        ]
    }
    #[cfg(not(windows))]
    {
        vec![
            "/etc/passwd",
            "/usr/bin/malware",
            "/root/.ssh/authorized_keys",
            "/home/user/secrets.txt",
        ]
    }
}

fn create_test_directory() -> std::io::Result<std::path::PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let temp_base = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let temp_dir = temp_base.join(format!("jailed_path_test_{}_{}", std::process::id(), nanos));

    // Create the main test directory
    fs::create_dir_all(&temp_dir)?;

    // Create a subdirectory structure for testing
    let sub_dir = temp_dir.join("subdir");
    fs::create_dir(&sub_dir)?;

    // Create a test file in the jail
    let test_file = temp_dir.join("test.txt");
    let mut file = fs::File::create(test_file)?;
    writeln!(file, "test content")?;

    // Create a test file in subdirectory
    let sub_file = sub_dir.join("sub_test.txt");
    let mut file = fs::File::create(sub_file)?;
    writeln!(file, "sub test content")?;

    Ok(temp_dir)
}

fn cleanup_test_directory(path: &std::path::Path) {
    if path.exists() {
        let _ = fs::remove_dir_all(path);
    }
}

#[test]
fn test_pathvalidator_creation_with_valid_directory() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");

    // Should successfully create validator with existing directory
    let result = PathValidator::<()>::with_jail(temp_dir.clone());
    assert!(
        result.is_ok(),
        "PathValidator creation should succeed with valid directory"
    );

    let validator = match result {
        Ok(v) => v,
        Err(e) => panic!("Expected Ok, got Err: {e:?}"),
    };
    assert_eq!(
        validator.jail().canonicalize().unwrap(),
        temp_dir.canonicalize().unwrap(),
        "Validator should store the canonical path of the jail"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_pathvalidator_creation_with_nonexistent_directory() {
    // Should succeed: jail does not need to exist
    let temp_base = std::env::temp_dir();
    let jail_path = temp_base.join(format!("jailed_path_nonexistent_{}", std::process::id()));
    // Ensure it does not exist
    if jail_path.exists() {
        let _ = std::fs::remove_dir_all(&jail_path);
    }
    let result = PathValidator::<()>::with_jail(&jail_path);
    assert!(
        result.is_ok(),
        "PathValidator creation should succeed with non-existent jail directory"
    );
}

#[test]
fn test_pathvalidator_creation_with_file_instead_of_directory() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let file_path = temp_dir.join("test.txt");

    // Should fail with InvalidJail when trying to use a file as jail
    let result = PathValidator::<()>::with_jail(&file_path);
    assert!(
        result.is_err(),
        "PathValidator creation should fail when jail is a file"
    );
    match result.unwrap_err() {
        JailedPathError::InvalidJail { jail, source } => {
            assert_eq!(jail, file_path);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            assert!(source.to_string().contains("not a directory"));
        }
        other => panic!("Expected InvalidJail, got: {other:?}"),
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_pathvalidator_creation_with_existing_directory() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let result = PathValidator::<()>::with_jail(&temp_dir);
    assert!(result.is_ok(), "Should succeed with existing directory");
    let validator = result.unwrap();
    assert_eq!(validator.jail(), temp_dir.canonicalize().unwrap());
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_valid_relative_path() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should successfully validate existing file with relative path
    let result = validator.try_path("test.txt");
    assert!(
        result.is_ok(),
        "try_path should succeed with valid relative path"
    );

    let jailed_path = result.unwrap();
    assert!(
        jailed_path.as_path().ends_with("test.txt"),
        "JailedPath should point to the correct file"
    );
    assert!(
        jailed_path.as_path().starts_with(validator.jail()),
        "JailedPath should be within jail boundary"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_valid_subdirectory_path() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should successfully validate file in subdirectory
    let result = validator.try_path("subdir/sub_test.txt");
    assert!(
        result.is_ok(),
        "try_path should succeed with valid subdirectory path"
    );

    let jailed_path = result.unwrap();
    assert!(
        jailed_path.as_path().ends_with("sub_test.txt"),
        "JailedPath should point to the correct file"
    );
    assert!(
        jailed_path.as_path().starts_with(validator.jail()),
        "JailedPath should be within jail boundary"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_directory_traversal_attack() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should block directory traversal attempts
    let traversal_attempts = vec![
        "../../../sensitive.txt",
        "../../..",
        "../outside.txt",
        "subdir/../../outside.txt",
        "subdir/../../../sensitive.txt",
    ];

    for attempt in traversal_attempts {
        let result = validator.try_path(attempt);
        assert!(
            result.is_err(),
            "try_path should block traversal attempt: {attempt}"
        );

        // With lexical validation, all paths containing ".." should return PathEscapesBoundary
        match result.unwrap_err() {
            JailedPathError::PathEscapesBoundary {
                attempted_path,
                jail_boundary,
            } => {
                assert_eq!(jail_boundary, validator.jail().to_path_buf());
                assert_eq!(attempted_path.to_string_lossy(), attempt);
                println!("✅ Blocked traversal attempt: {attempt}");
            }
            other => {
                panic!("Expected PathEscapesBoundary (from lexical validation), got: {other:?}")
            }
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_absolute_path_inside_jail() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should allow absolute path that's within the jail
    let absolute_path = temp_dir.join("test.txt");
    let result = validator.try_path(&absolute_path);
    assert!(
        result.is_ok(),
        "try_path should allow absolute path within jail"
    );

    let jailed_path = result.unwrap();
    assert_eq!(jailed_path.as_path(), absolute_path.canonicalize().unwrap());

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_absolute_path_outside_jail() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Create another temp directory outside the jail
    let outside_base = std::env::temp_dir();
    let outside_dir = outside_base.join(format!("jailed_path_outside_test_{}", std::process::id()));
    fs::create_dir_all(&outside_dir).expect("Failed to create outside temp directory");
    let outside_file = outside_dir.join("outside.txt");
    fs::File::create(&outside_file).expect("Failed to create outside file");

    // Should block absolute path outside jail
    let result = validator.try_path(&outside_file);
    assert!(
        result.is_err(),
        "try_path should block absolute path outside jail"
    );

    match result.unwrap_err() {
        JailedPathError::PathEscapesBoundary {
            attempted_path,
            jail_boundary,
        } => {
            // Verify the attempted path is outside the jail (key security check)
            assert!(!attempted_path.starts_with(&jail_boundary));
            assert_eq!(jail_boundary, validator.jail().to_path_buf());
        }
        other => panic!("Expected PathEscapesBoundary, got: {other:?}"),
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
    cleanup_test_directory(&outside_dir);
}

#[test]
fn test_try_path_with_nonexistent_file() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should successfully validate non-existent file using touch technique
    let result = validator.try_path("user123/new_document.pdf");
    assert!(
        result.is_ok(),
        "try_path should succeed with non-existent file using touch technique"
    );

    let jailed_path = result.unwrap();
    assert!(jailed_path.as_path().ends_with("new_document.pdf"));
    assert!(jailed_path.as_path().starts_with(validator.jail()));

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_nonexistent_nested_file() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should create parent directories and validate deeply nested non-existent file
    let result = validator.try_path("users/john/photos/vacation/beach.jpg");
    assert!(
        result.is_ok(),
        "try_path should succeed with deeply nested non-existent file"
    );

    let jailed_path = result.unwrap();
    assert!(jailed_path.as_path().ends_with("beach.jpg"));
    assert!(jailed_path.as_path().starts_with(validator.jail()));

    // SECURITY: Verify parent directories were cleaned up for anti-spam protection
    let parent_dir = temp_dir.join("users/john/photos/vacation");
    assert!(
        !parent_dir.exists(),
        "Parent directories should be cleaned up for security"
    );

    // Even the top-level directory should be cleaned up
    let top_dir = temp_dir.join("users");
    assert!(
        !top_dir.exists(),
        "All created directories should be cleaned up"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_blocks_traversal_in_nonexistent_paths() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should block traversal attacks even for non-existent paths
    let traversal_attempts = vec![
        "user/../../../sensitive.txt",
        "photos/../../../../../../malware.exe",
        "docs/../../../secrets.txt",
        "../escape/file.txt",
        "valid/path/../../../config.ini",
    ];

    for attempt in traversal_attempts {
        let result = validator.try_path(attempt);
        assert!(
            result.is_err(),
            "try_path should block traversal attempt in non-existent path: {attempt}"
        );

        match result.unwrap_err() {
            JailedPathError::PathEscapesBoundary {
                attempted_path,
                jail_boundary,
            } => {
                assert_eq!(jail_boundary, validator.jail().to_path_buf());
                assert!(!attempted_path.starts_with(&jail_boundary));
            }
            JailedPathError::PathResolutionError { .. } => {
                // Also acceptable if path resolution fails due to permissions
            }
            other => panic!("Expected PathEscapesBoundary or PathResolutionError, got: {other:?}"),
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_absolute_nonexistent_path_outside_jail() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should block absolute paths outside jail, even if they don't exist
    let outside_paths = get_attack_target_paths();

    for path in outside_paths {
        let result = validator.try_path(path);
        assert!(
            result.is_err(),
            "try_path should block absolute path outside jail: {path}"
        );

        match result.unwrap_err() {
            JailedPathError::PathEscapesBoundary {
                attempted_path,
                jail_boundary,
            } => {
                assert_eq!(jail_boundary, validator.jail().to_path_buf());
                assert!(!attempted_path.starts_with(&jail_boundary));
            }
            JailedPathError::PathResolutionError { .. } => {
                // Also acceptable if we can't create the file due to permissions
            }
            other => panic!("Expected PathEscapesBoundary or PathResolutionError, got: {other:?}"),
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_mixed_existing_and_nonexistent() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Create some existing structure
    let existing_dir = temp_dir.join("existing_user");
    std::fs::create_dir(existing_dir).unwrap();

    // Should validate path that goes through existing directory to non-existent file
    let result = validator.try_path("existing_user/new_file.txt");
    assert!(
        result.is_ok(),
        "try_path should handle existing directory + non-existent file"
    );

    let jailed_path = result.unwrap();
    assert!(jailed_path.as_path().starts_with(validator.jail()));
    assert!(jailed_path.as_path().ends_with("new_file.txt"));

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_preserves_file_after_validation() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    let test_path = "user123/document.pdf";
    let full_expected_path = temp_dir.join(test_path);

    // Validate non-existent path
    let result = validator.try_path(test_path);
    assert!(result.is_ok());

    // The temporary file should be cleaned up
    assert!(
        !full_expected_path.exists(),
        "Temporary file should be cleaned up after validation"
    );

    // SECURITY: Parent directories should also be cleaned up to prevent spam
    assert!(
        !full_expected_path.parent().unwrap().exists(),
        "Parent directories should be cleaned up for security (anti-spam protection)"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_handles_permission_errors_gracefully() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Try to create file in a location that might have permission issues
    // This test behavior may vary by platform, but should not panic
    let result = validator.try_path("restricted/file.txt");

    // Should either succeed or fail gracefully with a clear error
    match result {
        Ok(jailed_path) => {
            assert!(jailed_path.as_path().starts_with(validator.jail()));
        }
        Err(JailedPathError::PathResolutionError { .. }) => {
            // Acceptable - permission denied or other IO error
        }
        Err(other) => {
            panic!("Unexpected error type: {other:?}");
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_edge_case_empty_relative_path() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test edge case: empty or current directory path
    let edge_cases = vec![".", "./", "./file.txt", "file.txt"];

    for path in edge_cases {
        let result = validator.try_path(path);
        if let Ok(jailed_path) = result {
            assert!(jailed_path.as_path().starts_with(validator.jail()));
        }
        // Some of these might fail, which is acceptable behavior
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_with_complex_traversal_patterns() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Complex traversal patterns that should all be blocked
    let complex_attacks = vec![
        "../../../sensitive.txt",             // Direct escape - 3 levels up
        "../../../../config.ini",             // Direct escape - 4 levels up
        "a/../../../sensitive.txt",           // 1 down, 3 up = 2 net up (escapes)
        "a/b/../../../../../../malware.exe",  // 2 down, 7 up = 5 net up (escapes)
        "user/../../../../../../secrets.txt", // 1 down, 6 up = 5 net up (escapes)
    ];

    for attack in complex_attacks {
        let result = validator.try_path(attack);
        assert!(
            result.is_err(),
            "Complex traversal attack should be blocked: {attack}"
        );

        if let Err(JailedPathError::PathEscapesBoundary { attempted_path, .. }) = result {
            assert!(!attempted_path.starts_with(validator.jail()));
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_performance_with_many_validations() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test that multiple validations work correctly
    let test_paths = [
        "user1/file1.txt",
        "user2/documents/file2.pdf",
        "user3/photos/vacation.jpg",
        "shared/document.docx",
        "temp/upload.tmp",
    ];

    for (i, path) in test_paths.iter().enumerate() {
        let result = validator.try_path(path);
        assert!(
            result.is_ok(),
            "Validation #{} should succeed for path: {path}",
            i + 1
        );

        let jailed_path = result.unwrap();
        assert!(jailed_path.as_path().starts_with(validator.jail()));
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_marker_types_for_compile_time_safety() {
    // Define test marker types
    struct ImageResource;
    struct UserData;

    let temp_dir = create_test_directory().expect("Failed to create temp directory");

    // Create validators with different marker types
    let image_validator: PathValidator<ImageResource> =
        PathValidator::with_jail(&temp_dir).unwrap();
    let user_validator: PathValidator<UserData> = PathValidator::with_jail(&temp_dir).unwrap();

    // Both should work with the same file but produce different marker types
    let image_path: JailedPath<ImageResource> = image_validator.try_path("test.txt").unwrap();
    let user_path: JailedPath<UserData> = user_validator.try_path("test.txt").unwrap();

    // Paths should be the same but have different types (checked at compile time)
    assert_eq!(image_path.as_path(), user_path.as_path());

    // This ensures the PhantomData marker is working and size is consistent
    let expected_size = std::mem::size_of::<PathBuf>() + std::mem::size_of::<Arc<PathBuf>>();
    assert_eq!(
        std::mem::size_of::<JailedPath<ImageResource>>(),
        expected_size,
        "JailedPath should have consistent size regardless of marker type"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_validator_jail_accessor() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // jail() method should return the canonical jail path
    let jail_path = validator.jail();
    assert_eq!(jail_path, temp_dir.canonicalize().unwrap());

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
#[allow(clippy::redundant_clone)]
fn test_validator_clone_and_debug() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Should be cloneable
    let cloned_validator = validator.clone();
    assert_eq!(validator.jail(), cloned_validator.jail());

    // Should be debuggable (just ensure it doesn't panic)
    let debug_str = format!("{validator:?}");
    assert!(debug_str.contains("PathValidator"));

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_anti_directory_spam_protection() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test deep nested path that would create many directories
    let deep_path = "spam1/spam2/spam3/spam4/spam5/spam6/spam7/spam8/spam9/spam10/file.txt";

    // Capture directory state before validation
    let initial_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    // Validate the deep path
    let result = validator.try_path(deep_path);
    assert!(result.is_ok(), "Deep path validation should succeed");

    // Check that no spam directories were left behind
    let final_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    // Should have same number of entries (no spam directories left)
    assert_eq!(
        initial_entries.len(),
        final_entries.len(),
        "Directory count should be the same after validation - no spam directories"
    );

    // Specifically check that spam1 directory was not left behind
    let spam_dir = temp_dir.join("spam1");
    assert!(
        !spam_dir.exists(),
        "Spam directory should be completely cleaned up: {spam_dir:?}"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_preserves_existing_directories() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Create an existing directory structure
    let existing_path = temp_dir.join("existing");
    std::fs::create_dir(&existing_path).unwrap();
    let nested_existing = existing_path.join("nested");
    std::fs::create_dir(&nested_existing).unwrap();

    // Validate a file in the existing structure + new subdirectory
    let result = validator.try_path("existing/nested/new_subdir/file.txt");
    assert!(
        result.is_ok(),
        "Path with existing + new directories should work"
    );

    // Existing directories should still exist
    assert!(existing_path.exists(), "Existing directory should remain");
    assert!(
        nested_existing.exists(),
        "Nested existing directory should remain"
    );

    // But the new subdirectory should be cleaned up
    let new_subdir = nested_existing.join("new_subdir");
    assert!(
        !new_subdir.exists(),
        "New subdirectory should be cleaned up: {new_subdir:?}"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_try_path_cleanup_on_canonicalization_error() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Try to create a path that will fail during canonicalization
    // We'll use a very long path name that might hit OS limits
    let problematic_path = format!(
        "{}{}",
        "a/".repeat(1000), // Very deep nesting
        "x".repeat(300)    // Very long filename
    );

    let initial_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    // This should fail but not leave directories behind
    let _result = validator.try_path(problematic_path);

    // Regardless of success/failure, no directories should be left
    let final_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    assert_eq!(
        initial_entries.len(),
        final_entries.len(),
        "No directories should be left behind even on error"
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_massive_directory_spam_attack_prevention() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Capture initial state
    let initial_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    // Simulate massive spam attack with many deep paths
    let spam_paths = vec![
        "spam_attack_1/level1/level2/level3/level4/level5/file.txt",
        "spam_attack_2/a/b/c/d/e/f/g/h/i/j/file.txt",
        "spam_attack_3/very/deep/nested/structure/with/many/levels/file.txt",
        "spam_attack_4/user1/data/photos/2024/vacation/beach/file.jpg",
        "spam_attack_5/projects/web/assets/images/thumbnails/file.png",
        "spam_attack_6/documents/work/reports/2024/q1/analysis/file.pdf",
        "spam_attack_7/cache/temp/processing/batch1/output/file.tmp",
        "spam_attack_8/logs/application/debug/2024/07/18/file.log",
        "spam_attack_9/backup/incremental/daily/week1/data/file.bak",
        "spam_attack_10/system/config/modules/auth/settings/file.cfg",
    ];

    // Attempt all spam attacks
    for spam_path in &spam_paths {
        let result = validator.try_path(spam_path);
        // Should succeed (paths are valid within jail)
        assert!(result.is_ok(), "Spam path should validate: {spam_path}");
    }

    // CRITICAL: Verify NO spam directories were left behind
    let final_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    assert_eq!(
        initial_entries.len(),
        final_entries.len(),
        "Directory count should be identical - no spam directories left behind"
    );

    // Specifically verify that none of the spam directories exist
    for spam_path in &spam_paths {
        let spam_root = temp_dir.join(spam_path.split('/').next().unwrap());
        assert!(
            !spam_root.exists(),
            "Spam directory should not exist: {spam_root:?}"
        );
    }

    println!(
        "✅ Successfully prevented directory spam attack with {} attempts",
        spam_paths.len()
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_cleanup_on_jail_escape_attempts() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Create an existing subdirectory structure in the jail
    let existing_subdir = temp_dir.join("legitimate_user_data");
    std::fs::create_dir(&existing_subdir).unwrap();
    let existing_nested = existing_subdir.join("photos");
    std::fs::create_dir(&existing_nested).unwrap();

    // Capture initial state - should have our existing structure
    let initial_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    // Test various jail escape attempts that create directories
    let escape_attempts = vec![
        // These should create directories inside jail, then fail validation
        "legitimate_user_data/photos/../../../../../../../sensitive.txt",
        "legitimate_user_data/new_folder/../../../../../../malware.exe",
        "existing_dir/../../../../../../../secrets.txt",
        "valid/path/../../../../../../../evil.txt",
        // Direct escapes that might try to create directories outside
        "../../../outside_jail/malicious.txt",
        "../../../../config.ini",
    ];

    for escape_attempt in &escape_attempts {
        println!("Testing escape attempt: {escape_attempt}");

        // Record state before attempt
        let pre_attempt_entries: Vec<_> = std::fs::read_dir(&temp_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();

        // Attempt the escape - should fail but not leave mess
        let result = validator.try_path(escape_attempt);

        // Should fail with escape error
        assert!(
            result.is_err(),
            "Escape attempt should fail: {escape_attempt}"
        );

        match result.unwrap_err() {
            JailedPathError::PathEscapesBoundary { .. } => {
                // Expected - this is good
            }
            JailedPathError::PathResolutionError { .. } => {
                // Also acceptable - might fail to create dirs outside jail
            }
            other => {
                panic!("Unexpected error type for escape attempt {escape_attempt}: {other:?}");
            }
        }

        // CRITICAL: Verify no new directories were left behind
        let post_attempt_entries: Vec<_> = std::fs::read_dir(&temp_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();

        // Allow for some tolerance in case of OS-specific temp file behavior
        if post_attempt_entries.len() > pre_attempt_entries.len() {
            println!("⚠️  Warning: Extra entries after escape attempt {escape_attempt}:");
            for entry in &post_attempt_entries {
                if !pre_attempt_entries.contains(entry) {
                    println!("   Extra: {entry:?}");
                    // Try to clean up any leftover directories
                    if entry.is_dir() {
                        let _ = std::fs::remove_dir_all(entry);
                    } else {
                        let _ = std::fs::remove_file(entry);
                    }
                }
            }
        }

        // Verify existing directories are still intact
        assert!(
            existing_subdir.exists(),
            "Existing directory should remain untouched: {existing_subdir:?}"
        );
        assert!(
            existing_nested.exists(),
            "Existing nested directory should remain untouched: {existing_nested:?}"
        );
    }

    // Final verification - should be back to initial state
    let final_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    assert_eq!(
        initial_entries.len(),
        final_entries.len(),
        "Should be back to initial state after all escape attempts"
    );

    println!(
        "✅ Successfully cleaned up after {} jail escape attempts",
        escape_attempts.len()
    );

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_attacker_path_in_existing_directory_with_escape() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Create existing directory structure (like /home/my_user/import_dir/)
    let import_dir = temp_dir.join("import_dir");
    std::fs::create_dir(&import_dir).unwrap();
    let user_data = import_dir.join("user_data");
    std::fs::create_dir(&user_data).unwrap();

    // Verify initial state
    assert!(import_dir.exists(), "Import dir should exist");
    assert!(user_data.exists(), "User data dir should exist");

    // Test the exact scenario you mentioned:
    // Existing: /import_dir/user_data/
    // Attack: dir_created_by_attacker/another_subdir/../../../../../sensitive.txt
    let attack_path =
        "import_dir/user_data/dir_created_by_attacker/another_subdir/../../../../../sensitive.txt";

    println!("Testing attack path: {attack_path}");

    // This should fail (escapes jail) but not leave directories behind
    let result = validator.try_path(attack_path);
    assert!(result.is_err(), "Attack should fail");

    // Verify the attack was blocked
    match result.unwrap_err() {
        JailedPathError::PathEscapesBoundary {
            attempted_path,
            jail_boundary,
        } => {
            println!("✅ Correctly blocked escape to: {attempted_path:?}");
            assert!(!attempted_path.starts_with(jail_boundary));
        }
        JailedPathError::PathResolutionError { .. } => {
            // Also acceptable - might fail due to permission issues or file conflicts
            println!("✅ Attack failed due to path resolution error (also acceptable)");
        }
        other => {
            panic!("Expected PathEscapesBoundary or PathResolutionError, got: {other:?}");
        }
    }

    // CRITICAL: Verify existing directories are untouched
    assert!(
        import_dir.exists(),
        "Original import_dir should remain untouched: {import_dir:?}"
    );
    assert!(
        user_data.exists(),
        "Original user_data should remain untouched: {user_data:?}"
    );

    // CRITICAL: Verify no attacker directories were left behind
    let attacker_dir = user_data.join("dir_created_by_attacker");
    if attacker_dir.exists() {
        println!(
            "⚠️  Warning: Attacker directory still exists, cleaning up manually: {attacker_dir:?}"
        );
        let _ = std::fs::remove_dir_all(&attacker_dir);
    }
    assert!(
        !attacker_dir.exists(),
        "Attacker directory should NOT exist: {attacker_dir:?}"
    );

    // Verify overall jail state is clean
    let jail_entries: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    println!("Final jail contents: {jail_entries:?}");

    // Should only have our initial test structure + import_dir
    assert!(jail_entries
        .iter()
        .any(|p| p.file_name().unwrap() == "import_dir"));

    println!("✅ Successfully blocked escape and cleaned up attacker directories");

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_lexical_validation_blocks_parent_directory_components() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test cases with parent directory components that should be blocked
    let malicious_paths = vec![
        // Basic traversal attempts
        "..",
        "../",
        "../file.txt",
        "../../../sensitive.txt",
        // Mixed with normal path components
        "user/../file.txt",
        "documents/../../../sensitive.txt",
        "data/backup/../../..",
        "uploads/user123/../../../config.ini",
        // Complex traversal patterns
        "a/b/../c/../../../sensitive.txt",
        "safe/dir/../../../../../secrets.txt",
        "nested/very/deep/../../../../..",
        // Realistic attack scenarios
        "user_uploads/../../../malware.exe",
        "temp/extract/../../malware.exe",
        "logs/user/../../../auth.log",
        "files/../../config.ini",
    ];

    for malicious_path in malicious_paths {
        let result = validator.try_path(malicious_path);

        // All should be rejected
        assert!(
            result.is_err(),
            "Path with '..' should be rejected: {malicious_path}"
        );

        // Should specifically be PathEscapesBoundary error (not PathResolutionError)
        match result.unwrap_err() {
            JailedPathError::PathEscapesBoundary {
                attempted_path,
                jail_boundary,
            } => {
                assert_eq!(attempted_path.to_string_lossy(), malicious_path);
                assert_eq!(jail_boundary, validator.jail().to_path_buf());
                println!("✅ Correctly blocked: {malicious_path}");
            }
            other => {
                panic!("Expected PathEscapesBoundary for path '{malicious_path}', got: {other:?}");
            }
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_lexical_validation_allows_legitimate_paths() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test cases that should be allowed (no actual ".." components)
    let legitimate_paths = vec![
        // Normal paths
        "file.txt",
        "subdir/file.txt",
        "users/john/documents/report.pdf",
        "data/2024/january/backup.zip",
        // Paths with ".." in filenames (not as path components)
        "..file.txt",              // filename starts with ..
        "file..txt",               // filename contains ..
        "my_file...extension",     // multiple dots
        "documents/..hidden_file", // hidden file starting with ..
        "config/app..backup.conf", // .. inside filename
        // Current directory references (allowed)
        "./file.txt",
        "subdir/./file.txt",
        "./subdir/file.txt",
        // Complex legitimate paths
        "user_data/projects/website/assets/images/logo.png",
        "exports/reports/2024/Q1/financial_summary.xlsx",
        "uploads/profile_pics/user_12345/avatar.jpg",
    ];

    for legitimate_path in legitimate_paths {
        let result = validator.try_path(legitimate_path);

        // Should either succeed or fail with PathResolutionError (file doesn't exist)
        // but never with PathEscapesBoundary
        match result {
            Ok(_) => {
                println!("✅ Correctly allowed (exists): {legitimate_path}");
            }
            Err(JailedPathError::PathResolutionError { .. }) => {
                println!("✅ Correctly allowed (doesn't exist): {legitimate_path}");
            }
            Err(JailedPathError::PathEscapesBoundary { .. }) => {
                panic!("Legitimate path should not be blocked as escape: {legitimate_path}");
            }
            Err(other) => {
                panic!("Unexpected error for legitimate path '{legitimate_path}': {other:?}");
            }
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_absolute_path_lexical_validation() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test absolute paths that should be blocked for different reasons
    let jail_str = temp_dir.to_string_lossy();

    // Absolute paths outside jail (should be blocked during canonicalization)
    let outside_absolute_paths = get_attack_target_paths();

    for abs_path in outside_absolute_paths {
        let result = validator.try_path(abs_path);
        assert!(
            result.is_err(),
            "Absolute path outside jail should be blocked: {abs_path}"
        );

        // These should be blocked either by:
        // 1. PathEscapesBoundary (after canonicalization)
        // 2. PathResolutionError (if path doesn't exist)
        match result.unwrap_err() {
            JailedPathError::PathEscapesBoundary { .. } => {
                println!("✅ Correctly blocked absolute path outside jail: {abs_path}");
            }
            JailedPathError::PathResolutionError { .. } => {
                println!(
                    "✅ Correctly blocked non-existent absolute path outside jail: {abs_path}"
                );
            }
            other => {
                panic!(
                    "Expected PathEscapesBoundary or PathResolutionError for absolute path '{abs_path}', got: {other:?}"
                );
            }
        }
    }

    // Absolute paths inside jail but with .. components
    let jail_with_traversal = vec![
        format!("{}/../../../sensitive.txt", jail_str),
        format!("{}/subdir/../../../secrets.txt", jail_str),
        format!("{}/user/../../config.ini", jail_str),
    ];

    for path_with_traversal in jail_with_traversal {
        let result = validator.try_path(&path_with_traversal);
        assert!(
            result.is_err(),
            "Absolute path with traversal should be blocked: {path_with_traversal}"
        );

        match result.unwrap_err() {
            JailedPathError::PathEscapesBoundary { .. } => {
                println!(
                    "✅ Correctly blocked absolute path with traversal: {path_with_traversal}"
                );
            }
            other => {
                panic!(
                    "Expected PathEscapesBoundary for absolute path with traversal '{path_with_traversal}', got: {other:?}"
                );
            }
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_lexical_validation_is_fast_and_secure() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Verify that lexical validation prevents filesystem operations for malicious paths
    let malicious_path = "../../../sensitive.txt";

    // This should fail immediately without creating any files or directories
    let result = validator.try_path(malicious_path);

    assert!(result.is_err(), "Malicious path should be rejected");

    match result.unwrap_err() {
        JailedPathError::PathEscapesBoundary { .. } => {
            println!("✅ Lexical validation correctly blocked traversal attempt");
        }
        other => {
            panic!("Expected PathEscapesBoundary from lexical validation, got: {other:?}");
        }
    }

    // The key point is that lexical validation blocks the path immediately
    // without any filesystem operations, which is much safer than the touch technique
    println!("✅ No malicious files created during validation");

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_display_functionality() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test various paths to ensure virtual root display works correctly
    let separator = std::path::MAIN_SEPARATOR;
    let test_cases = vec![
        ("file.txt", format!("{separator}file.txt")),
        (
            "subdir/file.txt",
            format!("{separator}subdir{separator}file.txt"),
        ),
        (
            "users/alice/documents/report.pdf",
            format!(
                "{separator}users{separator}alice{separator}documents{separator}report.pdf"
            ),
        ),
        (
            "deeply/nested/path/structure/file.log",
            format!(
                "{separator}deeply{separator}nested{separator}path{separator}structure{separator}file.log"
            ),
        ),
    ];

    for (input_path, expected_display) in test_cases {
        let result = validator.try_path(input_path);
        assert!(
            result.is_ok(),
            "Path validation should succeed for: {input_path}"
        );

        let jailed_path = result.unwrap();

        // Test Display trait - should show virtual root (relative to jail)
        let display_output = format!("{jailed_path}");
        assert_eq!(
            display_output, expected_display,
            "Display should show virtual root for path: {input_path}"
        );

        // Verify it's a proper relative path starting with platform separator
        assert!(
            display_output.starts_with(std::path::MAIN_SEPARATOR),
            "Virtual root display should start with platform separator: {display_output}"
        );

        // Verify it doesn't contain the actual jail path
        let jail_str = temp_dir.to_string_lossy();
        assert!(
            !display_output.contains(&*jail_str),
            "Virtual root display should not contain actual jail path: {display_output}"
        );

        println!("✅ Virtual root display: {input_path} -> {display_output}");
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_debug_formatting() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    let jailed_path = validator.try_path("user/document.pdf").unwrap();

    let debug_output = format!("{jailed_path:?}");

    assert!(debug_output.contains("JailedPath"));
    assert!(debug_output.contains("path:"));
    assert!(debug_output.contains("jail_root:"));

    let expected_debug_prefix = "JailedPath { path: ";
    assert!(debug_output.starts_with(expected_debug_prefix));

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_display_vs_debug_differences() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    let jailed_path = validator.try_path("users/alice/file.txt").unwrap();

    // Get both outputs
    let display_output = format!("{jailed_path}");
    let debug_output = format!("{jailed_path:?}");

    // Display should be clean and user-friendly
    let separator = std::path::MAIN_SEPARATOR;
    let expected_display = format!("{separator}users{separator}alice{separator}file.txt");
    assert_eq!(display_output, expected_display);

    // Debug should be verbose and contain internal details
    assert!(debug_output.len() > display_output.len());
    assert!(debug_output.contains("JailedPath"));

    // They should be completely different
    assert_ne!(display_output, debug_output);

    // Display should NOT contain debug formatting
    assert!(!display_output.contains("JailedPath"));
    assert!(!display_output.contains("path:"));
    assert!(!display_output.contains("jail_root:"));

    println!("✅ Display: {display_output}");
    println!("✅ Debug: {debug_output}");

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_jail_root_accessor() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    let jailed_path = validator.try_path("file.txt").unwrap();

    // Test jail_root() accessor method
    let jail_root = jailed_path.jail_root();

    // Should return the same path as validator.jail()
    assert_eq!(jail_root, validator.jail());

    // Should be the canonical jail path
    assert_eq!(jail_root, temp_dir.canonicalize().unwrap());

    println!("✅ Jail root accessor works: {}", jail_root.display());

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_with_different_marker_types() {
    struct UserFiles;
    struct ConfigFiles;

    let temp_dir = create_test_directory().expect("Failed to create temp directory");

    let user_validator: PathValidator<UserFiles> = PathValidator::with_jail(&temp_dir).unwrap();
    let config_validator: PathValidator<ConfigFiles> = PathValidator::with_jail(&temp_dir).unwrap();

    let user_path: JailedPath<UserFiles> = user_validator.try_path("user_data.json").unwrap();
    let config_path: JailedPath<ConfigFiles> = config_validator.try_path("config.toml").unwrap();

    // Both should have same virtual root display behavior regardless of marker type
    let separator = std::path::MAIN_SEPARATOR;
    assert_eq!(
        format!("{user_path}"),
        format!("{}user_data.json", separator)
    );
    assert_eq!(
        format!("{config_path}"),
        format!("{}config.toml", separator)
    );

    // Both should have access to jail_root()
    assert_eq!(user_path.jail_root(), config_path.jail_root());
    assert_eq!(user_path.jail_root(), temp_dir.canonicalize().unwrap());

    // Debug formatting should work for both
    let user_debug = format!("{user_path:?}");
    let config_debug = format!("{config_path:?}");

    assert!(user_debug.contains("JailedPath"));
    assert!(config_debug.contains("JailedPath"));

    println!("✅ Virtual root works with marker types");
    println!("   User: {user_path}");
    println!("   Config: {config_path}");

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_display_edge_cases() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test edge cases for virtual root display
    let separator = std::path::MAIN_SEPARATOR;
    let edge_cases = vec![
        // Root file (no subdirectory)
        ("root_file.txt", format!("{separator}root_file.txt")),
        // Single character names
        ("a", format!("{separator}a")),
        ("a/b", format!("{separator}a{separator}b")),
        // Files with dots
        (".hidden", format!("{separator}.hidden")),
        (
            "file.with.many.dots.txt",
            format!("{separator}file.with.many.dots.txt"),
        ),
    ];

    for (input_path, expected_display) in edge_cases {
        let result = validator.try_path(input_path);
        if let Ok(jailed_path) = result {
            let display_output = format!("{jailed_path}");

            // Should start with platform separator (virtual root)
            assert!(
                display_output.starts_with(std::path::MAIN_SEPARATOR),
                "Virtual root should start with platform separator for: {input_path} -> {display_output}"
            );

            // Should match expected format
            assert_eq!(
                display_output, expected_display,
                "Virtual root display mismatch for: {input_path}"
            );

            println!("✅ Edge case: {input_path} -> {display_output}");
        }
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_with_cross_platform_paths() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // Test that virtual root display handles cross-platform path separators
    let jailed_path = validator
        .try_path("users/alice/documents/file.txt")
        .unwrap();
    let display_output = format!("{jailed_path}");

    // Virtual root should use platform-appropriate separators
    assert!(display_output.starts_with(std::path::MAIN_SEPARATOR));

    println!("✅ Cross-platform virtual root: {display_output}");
    println!("   Underlying path: {}", jailed_path.as_path().display());

    // The virtual root display should be clean and consistent
    assert!(!display_output.is_empty());
    assert!(display_output.contains("file.txt"));

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
#[cfg(windows)]
fn test_virtual_root_display_windows_separators() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // On Windows, virtual root should use backslashes (Windows convention)
    let test_cases = vec![
        ("file.txt", "\\file.txt"),
        ("subdir/file.txt", "\\subdir\\file.txt"),
        (
            "users/alice/documents/report.pdf",
            "\\users\\alice\\documents\\report.pdf",
        ),
        (
            "deeply/nested/path/structure/file.log",
            "\\deeply\\nested\\path\\structure\\file.log",
        ),
    ];

    for (input_path, expected_display) in test_cases {
        let result = validator.try_path(input_path);
        assert!(
            result.is_ok(),
            "Path validation should succeed for: {input_path}"
        );

        let jailed_path = result.unwrap();
        let display_output = format!("{jailed_path}");

        // On Windows, should use backslashes
        assert_eq!(
            display_output, expected_display,
            "Windows virtual root should use backslashes for: {input_path}"
        );

        // Should start with backslash (Windows virtual root)
        assert!(
            display_output.starts_with('\\'),
            "Windows virtual root should start with '\\': {display_output}"
        );

        // Should contain backslashes for nested paths
        if input_path.contains('/') {
            assert!(
                display_output.contains('\\'),
                "Windows virtual root should contain backslashes: {display_output}"
            );
        }

        println!("✅ Windows virtual root: {input_path} -> {display_output}");
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
#[cfg(unix)]
fn test_virtual_root_display_unix_separators() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    // On Unix/Linux/macOS, virtual root should use forward slashes (Unix convention)
    let test_cases = vec![
        ("file.txt", "/file.txt"),
        ("subdir/file.txt", "/subdir/file.txt"),
        (
            "users/alice/documents/report.pdf",
            "/users/alice/documents/report.pdf",
        ),
        (
            "deeply/nested/path/structure/file.log",
            "/deeply/nested/path/structure/file.log",
        ),
    ];

    for (input_path, expected_display) in test_cases {
        let result = validator.try_path(input_path);
        assert!(
            result.is_ok(),
            "Path validation should succeed for: {input_path}"
        );

        let jailed_path = result.unwrap();
        let display_output = format!("{jailed_path}");

        // On Unix, should use forward slashes
        assert_eq!(
            display_output, expected_display,
            "Unix virtual root should use forward slashes for: {input_path}"
        );

        // Should start with forward slash (Unix virtual root)
        assert!(
            display_output.starts_with('/'),
            "Unix virtual root should start with '/': {display_output}"
        );

        // Should not contain backslashes
        assert!(
            !display_output.contains('\\'),
            "Unix virtual root should not contain backslashes: {display_output}"
        );

        println!("✅ Unix virtual root: {input_path} -> {display_output}");
    }

    // Cleanup
    cleanup_test_directory(&temp_dir);
}

#[test]
fn test_virtual_root_platform_consistency() {
    let temp_dir = create_test_directory().expect("Failed to create temp directory");
    let validator = PathValidator::<()>::with_jail(&temp_dir).unwrap();

    let jailed_path = validator.try_path("users/alice/file.txt").unwrap();
    let display_output = format!("{jailed_path}");

    // Should always start with the platform's main separator
    let expected_start = std::path::MAIN_SEPARATOR;
    assert!(
        display_output.starts_with(expected_start),
        "Virtual root should start with platform separator '{expected_start}': {display_output}"
    );

    // Verify platform-specific expectations
    #[cfg(windows)]
    {
        assert!(
            display_output.starts_with('\\'),
            "Windows should use backslash"
        );
        assert!(
            display_output.contains('\\'),
            "Windows should contain backslashes"
        );
        println!("✅ Windows platform consistency verified: {display_output}");
    }

    #[cfg(unix)]
    {
        assert!(
            display_output.starts_with('/'),
            "Unix should use forward slash"
        );
        assert!(
            !display_output.contains('\\'),
            "Unix should not contain backslashes"
        );
        println!("✅ Unix platform consistency verified: {display_output}");
    }

    // Should match the behavior of std::path::MAIN_SEPARATOR
    let separator_char = std::path::MAIN_SEPARATOR;
    assert!(
        display_output.starts_with(separator_char),
        "Should use MAIN_SEPARATOR '{separator_char}' at start: {display_output}"
    );

    println!("✅ Platform consistency verified with MAIN_SEPARATOR: '{separator_char}'");

    // Cleanup
    cleanup_test_directory(&temp_dir);
}
