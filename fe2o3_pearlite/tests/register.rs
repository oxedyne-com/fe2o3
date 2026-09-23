use oxedyne_fe2o3_pearlite::register::{
	self,
	Step,
	DESKTOP_ID,
	MIME_TYPE,
	PROG_ID,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;


#[test]
fn test_linux_plan_points_the_entry_at_the_binary() {
	let steps = register::plan_linux(Path::new("/opt/my apps/pearlite"), Path::new("/home/u/.local/share"));
	let desktop = steps.iter().find_map(|s| match s {
		Step::Write { path, body } if path.ends_with(DESKTOP_ID) => Some(body.clone()),
		_ => None,
	});
	match desktop {
		Some(b)	=> {
			assert!(b.contains("Exec=\"/opt/my apps/pearlite\" %f\n"), "{}", b);
			assert!(b.contains("MimeType=application/x-pearlite;"));
		},
		None	=> panic!("no .desktop entry in the plan"),
	}
	let last = steps.last().cloned();
	assert_eq!(last, Some(Step::Run {
		prog:		"xdg-mime".to_string(),
		args:		vec!["default".into(), DESKTOP_ID.into(), MIME_TYPE.into()],
		required:	true,
	}));
}

#[test]
fn test_desktop_exec_arg_escapes_reserved_characters() {
	assert_eq!(register::desktop_exec_arg(Path::new("/a/$b\"c`d\\e")), "\"/a/\\$b\\\"c\\`d\\\\e\"");
}

#[test]
fn test_windows_plan_writes_the_open_command() {
	let steps = register::plan_windows(Path::new("C:\\Tools\\pearlite-reader.exe"));
	let open = steps.iter().any(|s| match s {
		Step::Run { prog, args, .. } => prog == "reg"
			&& args.iter().any(|a| a == "HKCU\\Software\\Classes\\Pearlite.Document\\shell\\open\\command")
			&& args.iter().any(|a| a == "\"C:\\Tools\\pearlite-reader.exe\" \"%1\""),
		_ => false,
	});
	assert!(open);
	let ext = steps.iter().any(|s| match s {
		Step::Run { args, .. } => args.get(1).map(|k| k.ends_with("\\.prl")).unwrap_or(false)
			&& args.iter().any(|a| a == PROG_ID),
		_ => false,
	});
	assert!(ext);
}

#[test]
fn test_execute_writes_files_and_passes_an_optional_failure() {
	let dir = std::env::temp_dir().join(fmt!("pearlite-register-{}", std::process::id()));
	let steps = vec![
		Step::Write { path: dir.join("a").join("b.txt"), body: "hi".to_string() },
		Step::Run { prog: "pearlite-no-such-tool".to_string(), args: vec![], required: false },
	];
	let report = match register::execute(&steps) {
		Ok(r)	=> r,
		Err(e)	=> panic!("{}", e),
	};
	assert_eq!(report.len(), 2);
	assert!(report[1].starts_with("skipped"));
	assert_eq!(std::fs::read_to_string(dir.join("a").join("b.txt")).ok(), Some("hi".to_string()));
	let _ = std::fs::remove_dir_all(&dir);
	let failing = vec![Step::Run { prog: "pearlite-no-such-tool".to_string(), args: vec![], required: true }];
	assert!(register::execute(&failing).is_err());
}
