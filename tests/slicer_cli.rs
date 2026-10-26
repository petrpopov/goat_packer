use std::{ffi::OsStr, path::Path};

use goat_packer::slicer_cli::slice_command;

fn args(command: &std::process::Command) -> Vec<&OsStr> {
    command.get_args().collect()
}

#[test]
fn slice_command_disables_arrange_before_slicing() {
    let command = slice_command(
        Path::new("/opt/slicer"),
        &[],
        Path::new("/work/out"),
        Path::new("/work/plate.3mf"),
    );
    assert_eq!(command.get_program(), "/opt/slicer");
    assert_eq!(
        args(&command),
        ["--arrange", "0", "--slice", "1", "--outputdir", "/work/out", "/work/plate.3mf"]
    );
}

#[test]
fn slice_command_keeps_extra_flags_before_slice() {
    let command = slice_command(
        Path::new("/opt/slicer"),
        &["--no-check"],
        Path::new("/work/out"),
        Path::new("/work/plate.3mf"),
    );
    assert_eq!(
        args(&command),
        ["--arrange", "0", "--no-check", "--slice", "1", "--outputdir", "/work/out", "/work/plate.3mf"]
    );
}
