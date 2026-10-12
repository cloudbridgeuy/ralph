use super::*;

#[test]
fn test_spinner_default() {
    let spinner = Spinner::default();
    // Should not be running initially
    assert!(!spinner.is_running());
}

#[test]
fn test_spinner_new() {
    let spinner = Spinner::new();
    assert!(!spinner.is_running());
}

#[test]
fn test_spinner_with_enabled_false() {
    let spinner = Spinner::with_enabled(false);
    assert!(!spinner.is_enabled());
    assert!(!spinner.is_running());
}

#[test]
fn test_spinner_with_enabled_true() {
    let spinner = Spinner::with_enabled(true);
    assert!(spinner.is_enabled());
    assert!(!spinner.is_running());
}

#[test]
fn test_spinner_with_session_elapsed() {
    let spinner = Spinner::with_session_elapsed(5000);
    assert_eq!(spinner.session_elapsed_ms, 5000);
}

#[test]
fn test_spinner_disabled_start_stop() {
    // Disabled spinner should be a no-op
    let mut spinner = Spinner::with_enabled(false);
    spinner.start();
    assert!(!spinner.is_running()); // Should not actually start
    spinner.stop(); // Should be safe to call
}

#[test]
fn test_spinner_enabled_start_stop() {
    let mut spinner = Spinner::with_enabled(true);
    spinner.start();
    // Give thread a moment to start
    thread::sleep(Duration::from_millis(10));
    assert!(spinner.is_running());
    spinner.stop();
    assert!(!spinner.is_running());
}

#[test]
fn test_spinner_double_start() {
    let mut spinner = Spinner::with_enabled(true);
    spinner.start();
    spinner.start(); // Should be no-op
    thread::sleep(Duration::from_millis(10));
    assert!(spinner.is_running());
    spinner.stop();
}

#[test]
fn test_spinner_double_stop() {
    let mut spinner = Spinner::with_enabled(true);
    spinner.start();
    thread::sleep(Duration::from_millis(10));
    spinner.stop();
    spinner.stop(); // Should be safe to call multiple times
    assert!(!spinner.is_running());
}

#[test]
fn test_spinner_drop_stops() {
    let running = {
        let mut spinner = Spinner::with_enabled(true);
        spinner.start();
        thread::sleep(Duration::from_millis(10));
        Arc::clone(&spinner.running)
        // spinner dropped here
    };
    // Thread should have been stopped
    thread::sleep(Duration::from_millis(50));
    assert!(!running.load(Ordering::SeqCst));
}

#[test]
fn test_format_time_short_seconds() {
    assert_eq!(format_time_short(0), "0s");
    assert_eq!(format_time_short(1), "1s");
    assert_eq!(format_time_short(30), "30s");
    assert_eq!(format_time_short(59), "59s");
}

#[test]
fn test_format_time_short_minutes() {
    assert_eq!(format_time_short(60), "1m 0s");
    assert_eq!(format_time_short(90), "1m 30s");
    assert_eq!(format_time_short(125), "2m 5s");
    assert_eq!(format_time_short(3600), "60m 0s");
}

#[test]
fn test_format_spinner_time_iteration_only() {
    // When session time equals iteration time, show only iteration
    let result = format_spinner_time(12, 12_000);
    assert_eq!(result, "12s");
}

#[test]
fn test_format_spinner_time_with_session() {
    // When session time differs significantly, show both
    let result = format_spinner_time(12, 105_000);
    assert_eq!(result, "12s • Total: 1m 45s");
}

#[test]
fn test_format_spinner_time_small_difference() {
    // Small difference (1 second) should not show session time
    let result = format_spinner_time(12, 13_000);
    assert_eq!(result, "12s");
}

#[test]
fn test_spinner_iteration_elapsed() {
    let spinner = Spinner::with_enabled(false);
    thread::sleep(Duration::from_millis(50));
    let elapsed = spinner.iteration_elapsed_ms();
    assert!(elapsed >= 50);
    assert!(elapsed < 150); // Reasonable upper bound
}

#[test]
fn test_spinner_total_session_elapsed() {
    let mut spinner = Spinner::with_enabled(false);
    spinner.session_elapsed_ms = 10_000;
    thread::sleep(Duration::from_millis(50));
    let total = spinner.total_session_elapsed_ms();
    assert!(total >= 10_050);
    assert!(total < 10_150);
}

#[test]
fn test_spinner_accumulate_iteration_time() {
    let mut spinner = Spinner::with_enabled(false);
    thread::sleep(Duration::from_millis(100));
    spinner.accumulate_iteration_time();
    // Session elapsed should now have the iteration time
    assert!(spinner.session_elapsed_ms >= 100);
    assert!(spinner.session_elapsed_ms < 200);
    // New iteration should start fresh
    let elapsed = spinner.iteration_elapsed_ms();
    assert!(elapsed < 50); // Should be near zero
}

#[test]
fn test_spinner_chars_count() {
    // Verify we have enough spinner chars for smooth animation
    assert!(SPINNER_CHARS.len() >= 8); // Should have good variety (currently 10)
    assert_eq!(SPINNER_CHARS.len(), 10); // Document expected count
}

// Context-related tests

#[test]
fn test_spinner_context_default() {
    assert_eq!(
        SpinnerContext::default(),
        SpinnerContext::WaitingForResponse
    );
}

#[test]
fn test_spinner_context_messages() {
    assert_eq!(
        SpinnerContext::WaitingForResponse.message(),
        "Waiting for response..."
    );
    assert_eq!(SpinnerContext::Thinking.message(), "Thinking...");
    assert_eq!(SpinnerContext::WaitingForTool.message(), "Running tool...");
    assert_eq!(SpinnerContext::Buffering.message(), "Buffering code...");
    assert_eq!(
        SpinnerContext::Summarizing.message(),
        "Summarizing progress file..."
    );
}

#[test]
fn test_spinner_get_context_default() {
    let spinner = Spinner::with_enabled(false);
    assert_eq!(spinner.get_context(), SpinnerContext::WaitingForResponse);
}

#[test]
fn test_spinner_set_context() {
    let spinner = Spinner::with_enabled(false);
    assert_eq!(spinner.get_context(), SpinnerContext::WaitingForResponse);
    spinner.set_context(SpinnerContext::Thinking);
    assert_eq!(spinner.get_context(), SpinnerContext::Thinking);
    spinner.set_context(SpinnerContext::WaitingForTool);
    assert_eq!(spinner.get_context(), SpinnerContext::WaitingForTool);
}

#[test]
fn test_spinner_start_with_context() {
    let mut spinner = Spinner::with_enabled(true);
    spinner.start_with_context(SpinnerContext::Thinking);
    thread::sleep(Duration::from_millis(10));
    assert!(spinner.is_running());
    assert_eq!(spinner.get_context(), SpinnerContext::Thinking);
    spinner.stop();
    assert!(!spinner.is_running());
}

#[test]
fn test_spinner_context_changes_while_running() {
    let mut spinner = Spinner::with_enabled(true);
    spinner.start();
    thread::sleep(Duration::from_millis(10));
    assert!(spinner.is_running());

    // Change context while running
    spinner.set_context(SpinnerContext::WaitingForTool);
    assert_eq!(spinner.get_context(), SpinnerContext::WaitingForTool);

    // Change again
    spinner.set_context(SpinnerContext::Buffering);
    assert_eq!(spinner.get_context(), SpinnerContext::Buffering);

    spinner.stop();
}

#[test]
fn test_format_session_info_empty() {
    assert_eq!(format_session_info(&SpinnerSessionInfo::default()), "");
}

#[test]
fn test_format_session_info_persona() {
    let info = SpinnerSessionInfo {
        persona: Some("developer".to_string()),
    };
    assert_eq!(format_session_info(&info), "developer");
}

#[test]
fn test_spinner_with_session_context() {
    let info = SpinnerSessionInfo {
        persona: Some("chat".to_string()),
    };
    let spinner = Spinner::with_session_context(5000, info);
    assert_eq!(spinner.session_elapsed_ms, 5000);
    assert_eq!(spinner.session_info.persona, Some("chat".to_string()));
}
