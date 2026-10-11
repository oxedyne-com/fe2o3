//! A command given on the command line (`<app> server`) that refuses to run must fail the
//! process. Until 2026-10-11 an `Evaluation::Error` was dropped there, so a production start
//! missing its certificate exited with a nought and said nothing, and a supervisor with
//! `Restart=on-failure` read that as a deliberate stop (found staging Zoapedia, Z9).

use oxedyne_fe2o3_steel::app::tui::command_line_outcome;

use oxedyne_fe2o3_tui::lib_tui::repl::Evaluation;

#[test]
fn test_an_error_evaluation_fails_the_invocation() {
	let evals = vec![
		Evaluation::Output("starting".to_string()),
		Evaluation::Error("Missing certificate for vhost 'example.test'.".to_string()),
		Evaluation::Exit,
	];
	match command_line_outcome(evals, "Example") {
		Ok(())	=> panic!("an Error evaluation was swallowed"),
		Err(e)	=> {
			let msg = format!("{}", e);
			assert!(msg.contains("Missing certificate for vhost 'example.test'."), "{}", msg);
			assert!(msg.contains("Example"), "{}", msg);
		}
	}
}

#[test]
fn test_output_exit_and_none_succeed() {
	let evals = vec![
		Evaluation::Output("done".to_string()),
		Evaluation::None,
		Evaluation::Exit,
	];
	assert!(command_line_outcome(evals, "Example").is_ok());
}
