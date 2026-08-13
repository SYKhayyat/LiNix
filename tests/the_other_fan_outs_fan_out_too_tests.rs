//! `sbom` and `export` ask every manager one at a time.
//!
//! `latency.rs`'s `Shape` is a collapse detector for exactly this: overlap ≈ 1.0 with one wave
//! per child is the signature of a serial fan-out, and the floors were calibrated across three
//! platforms so they hold on a host nobody has measured. `the_fan_out_commands_still_fan_out`
//! drives it — and drives `list`, alone, under a name in the plural.
//!
//! Measured on Windows, 133 declarations, 17 live managers, debug build, quiet box, over the
//! same set of 21 child commands:
//!
//! ```text
//! shall list           3.59s wall   21 children summing to 20.99s   5.9x   1 wave
//! shall check health   3.47s wall   23 children summing to 22.03s   6.3x   2 waves
//! shall plan           3.68s wall   24 children summing to 14.03s   3.8x   3 waves
//! shall sbom          11.12s wall   21 children summing to 11.04s   1.0x  21 waves
//! shall export         9.42s wall   21 children summing to  9.36s   1.0x  21 waves
//! ```
//!
//! `list` overlaps the same twenty-one commands 5.9×. `sbom` and `export` overlap them not at
//! all — 1.0×, one wave each, the exact signature `Shape` names. Both fail *both* of its
//! bounds: 1.0 is under the 1.5 floor, and 21 waves is over a ceiling of 10.
//!
//! **The machinery is not missing, it is unused.** Nothing here asks for a new mechanism; the
//! concurrency that gives `list` its 5.9× is in the tree and these two commands do not reach
//! for it. On this host that is nine seconds a user waits for a report that a manager could
//! have answered in under two.
//!
//! **Why nothing caught it.** `Class::of` files both under `ConfigOnly` — *"Reads files,
//! answers, stops"* — and `Class::shape()` returns `None` for that class, so the detector never
//! looks at the two commands that trip it. The wall-clock budget the misfiling *does* give them
//! is 5s, which neither can meet: `shall sbom` prints its own `WARN … budgeted 5s` on an
//! ordinary run. A warning that fires every time is one users learn to scroll past, and it is
//! the same warning that exists to catch the next 98-second `info`.
//!
//! So this is one defect with two faces, and the second is the more expensive: the class table
//! is wrong, which simultaneously suppresses the check that would fail and enables one that
//! cries wolf.
//!
//! **The bounds are `Shape`'s, asked of the type.** Borrowed from `list` deliberately: these
//! commands carry no shape of their own, and that is the thing under test.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A config that declares what this host actually has.
///
/// **`adopt`, and not an empty `init`.** `list` fans out over the machine whatever the manifest
/// says, so the existing shape gate works from a bare directory. `sbom` and `export` report on
/// what is *declared*, so a bare directory makes them ask nothing at all and the gate skips
/// itself into a pass — which is the failure mode this file is about, arrived at from the other
/// side. Adoption gives them the manifest a user has on day one.
fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shall-fanout-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [&["init"][..], &["adopt", "-y"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_shall"))
            .args(args)
            .env("SHALL_CONFIG_DIR", dir.join("config"))
            .env("SHALL_DATA_DIR", dir.join("data"))
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the binary should run");
        assert!(out.status.success(), "`shall {}` failed", args.join(" "));
    }
    dir
}

/// The `Timings:` summary line, off stderr where `--timings` puts it.
///
/// Run inside the fixture: `export` writes its output file into the working directory, so a
/// child inheriting the test runner's cwd leaves `package.json` in the repository — five of
/// them, once the collision suffixes start.
fn timings_line(dir: &Path, subcommand: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(["--timings", subcommand])
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    report
        .lines()
        .find(|l| l.starts_with("Timings:"))
        .unwrap_or_else(|| {
            panic!(
                "`shall --timings {subcommand}` printed no `Timings:` line; the instrument this \
                 gate reads is gone:\n{report}"
            )
        })
        .to_string()
}

fn number_before(line: &str, unit: &str) -> Option<f64> {
    let at = line.find(unit)?;
    line[..at]
        .rsplit(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|t| !t.is_empty())?
        .parse()
        .ok()
}

/// One command against `Shape`, or a loud skip when the host cannot answer.
fn assert_not_serial(subcommand: &str) {
    let dir = fresh(subcommand);
    let line = timings_line(&dir, subcommand);

    let Some(children) = number_before(&line, " child command(s)") else {
        eprintln!("fan-out shape ({subcommand}): SKIPPED — no child commands here:\n  {line}");
        return;
    };
    let children = children as usize;
    let shape = shall::core::latency::Class::of("list")
        .shape()
        .expect("`list` asks every manager, so it carries a shape budget");
    if children < shape.min_children {
        eprintln!(
            "fan-out shape ({subcommand}): SKIPPED — {children} child command(s), too few for \
             an overlap ratio to mean anything:\n  {line}"
        );
        return;
    }

    let overlap = number_before(&line, "x overlap").expect("the line prints an overlap ratio");
    let waves = number_before(&line, " wave(s)").expect("the line prints a wave count") as usize;
    let ceiling = shape.wave_ceiling(children);

    assert!(
        overlap >= shape.min_overlap,
        "`shall {subcommand}` asked {children} managers and overlapped them {overlap:.1}x, under \
         the {:.1}x floor — that is asking them one at a time. `shall list` overlaps the same \
         managers on the same host several times over, so the concurrency exists and this \
         command does not use it.\n  {line}",
        shape.min_overlap
    );
    assert!(
        waves <= ceiling,
        "`shall {subcommand}` went quiet {} time(s) mid-run ({waves} waves over {children} \
         children, ceiling {ceiling}). One wave per child is the signature of a serial \
         run.\n  {line}",
        waves.saturating_sub(1)
    );
}

/// The control, and it must stay green: the same assertions over the command that already
/// fans out. If this ever goes red the host is the problem, not the two below it.
#[test]
fn list_still_fans_out_which_is_what_makes_the_rest_a_finding() {
    assert_not_serial("list");
}

#[test]
fn sbom_asks_the_managers_concurrently() {
    assert_not_serial("sbom");
}

#[test]
fn export_asks_the_managers_concurrently() {
    assert_not_serial("export");
}

/// The class table is wrong, and it says so in its own numbers.
///
/// `ConfigOnly` is documented as *"Reads files, answers, stops"* and every other command in it
/// is flat at ~150 ms whatever the manifest holds. These two spawn one child process per
/// manager. The label is what turns off the shape gate above and turns on a 5-second budget
/// they cannot meet, so it is not a naming quibble — it is the reason both halves misfire.
#[test]
fn a_command_that_spawns_a_child_per_manager_is_not_config_only() {
    use shall::core::latency::Class;

    let mut misfiled = Vec::new();
    for subcommand in ["sbom", "export"] {
        let dir = fresh(&format!("class-{subcommand}"));
        let line = timings_line(&dir, subcommand);
        let Some(children) = number_before(&line, " child command(s)") else {
            continue; // No managers on this host; the class cannot be judged from here.
        };
        if children as usize >= Class::of("list").shape().unwrap().min_children
            && Class::of(subcommand).shape().is_none()
        {
            misfiled.push(format!(
                "`{subcommand}` is {:?} and spawned {} child command(s):\n    {line}",
                Class::of(subcommand),
                children as usize
            ));
        }
    }

    assert!(
        misfiled.is_empty(),
        "{} command(s) ask every manager and are classed as reading only files, which is what \
         exempts them from the shape budget:\n  {}",
        misfiled.len(),
        misfiled.join("\n  ")
    );
}
