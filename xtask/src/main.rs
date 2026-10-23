// SPDX-License-Identifier: GPL-3.0-or-later

//! Build tasks that cargo alone can't express. Run with `cargo xtask <task>`.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Oldest macOS the app supports. Keep in sync with `platforms` in apps/macos/Package.swift.
const MACOS_DEPLOYMENT_TARGET: &str = "14.0";
const FFI_CRATE: &str = "branchosaurus-ffi";
const FFI_LIB: &str = "libbranchosaurus_ffi.a";
const APP_NAME: &str = "Branchosaurus";
/// Started by git for credential prompts; lives next to the app's executable.
const ASKPASS: &str = "branchosaurus-askpass";

const USAGE: &str = "\
Usage: cargo xtask <task> [options]

Tasks:
  swift-bindings   Build the Rust library and generate the Swift bindings into apps/macos
  macos-app        swift-bindings, then build target/macos/Branchosaurus.app
  macos-test       swift-bindings, then run the Swift tests in apps/macos

Options:
  --release        Optimized build
  --universal      Build for both arm64 and x86_64 (needs `rustup target add x86_64-apple-darwin
                   aarch64-apple-darwin`)
";

#[derive(Clone, Copy)]
struct Options {
    release: bool,
    universal: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = Options {
        release: args.iter().any(|arg| arg == "--release"),
        universal: args.iter().any(|arg| arg == "--universal"),
    };
    let result = match args.first().map(String::as_str) {
        Some("swift-bindings") => swift_bindings(options).map(|_| ()),
        Some("macos-app") => macos_app(options),
        Some("macos-test") => macos_test(options),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_owned()
}

fn macos_dir() -> PathBuf {
    workspace_root().join("apps").join("macos")
}

/// Where the macOS build puts the Rust library and the app bundle.
fn output_dir() -> PathBuf {
    workspace_root().join("target").join("macos")
}

fn run(command: &mut Command) -> Result {
    let status = command.status().map_err(|err| {
        format!(
            "could not run {}: {err}",
            command.get_program().to_string_lossy()
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} {} failed with {status}",
            command.get_program().to_string_lossy(),
            command
                .get_args()
                .map(OsStr::to_string_lossy)
                .collect::<Vec<_>>()
                .join(" ")
        )
        .into())
    }
}

fn cargo() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(workspace_root());
    command
}

/// Builds the static library (one or two architectures) and generates the bindings. Returns the
/// directory containing `libbranchosaurus_ffi.a`.
fn swift_bindings(options: Options) -> Result<PathBuf> {
    let profile = if options.release { "release" } else { "debug" };
    let targets: Vec<Option<&str>> = if options.universal {
        vec![Some("aarch64-apple-darwin"), Some("x86_64-apple-darwin")]
    } else {
        vec![None]
    };

    let mut libraries = Vec::new();
    for target in &targets {
        let mut build = cargo();
        build
            .args(["build", "--package", FFI_CRATE, "--lib"])
            .env("MACOSX_DEPLOYMENT_TARGET", MACOS_DEPLOYMENT_TARGET);
        if options.release {
            build.arg("--release");
        }
        if let Some(target) = target {
            build.args(["--target", target]);
        }
        run(&mut build)?;
        let mut library = workspace_root().join("target");
        if let Some(target) = target {
            library.push(target);
        }
        libraries.push(library.join(profile).join(FFI_LIB));
    }

    // Only the static library goes here: next to the .dylib, the linker would pick the dylib.
    let lib_dir = output_dir().join("lib");
    fs::create_dir_all(&lib_dir)?;
    let library = lib_dir.join(FFI_LIB);
    if libraries.len() == 1 {
        fs::copy(&libraries[0], &library)?;
    } else {
        run(Command::new("lipo")
            .arg("-create")
            .args(&libraries)
            .arg("-output")
            .arg(&library))?;
    }

    let generated = output_dir().join("bindings");
    if generated.exists() {
        fs::remove_dir_all(&generated)?;
    }
    run(cargo()
        .args([
            "run",
            "--quiet",
            "--package",
            FFI_CRATE,
            "--features",
            "bindgen",
            "--bin",
            "uniffi-bindgen-swift",
            "--",
        ])
        .arg(&library)
        .arg(&generated)
        .args([
            "--swift-sources",
            "--headers",
            "--modulemap",
            "--module-name",
            "BranchosaurusFFI",
            "--modulemap-filename",
            "module.modulemap",
        ]))?;

    let include_dir = macos_dir().join("Sources/BranchosaurusFFI/include");
    let swift_dir = macos_dir().join("Sources/BranchosaurusCore/Generated");
    for dir in [&include_dir, &swift_dir] {
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        fs::create_dir_all(dir)?;
    }
    for entry in fs::read_dir(&generated)? {
        let path = entry?.path();
        let destination = match path.extension().and_then(OsStr::to_str) {
            Some("swift") => &swift_dir,
            Some("h" | "modulemap") => &include_dir,
            _ => continue,
        };
        fs::copy(
            &path,
            destination.join(path.file_name().expect("file name")),
        )?;
    }
    println!("Swift bindings written to {}", macos_dir().display());
    Ok(lib_dir)
}

/// `swift build` in apps/macos, adjusted for machines that only have the Command Line Tools:
/// their copy of the default build engine needs Xcode, and SwiftUI in SDKs newer than 26 relies
/// on a macro plugin that only Xcode ships, so those builds use the older engine and SDK.
fn swift_build() -> Command {
    swift("build")
}

/// `swift <subcommand>` in apps/macos; see [`swift_build`].
fn swift(subcommand: &str) -> Command {
    let mut command = Command::new("swift");
    command.arg(subcommand).current_dir(macos_dir());
    let developer_dir = Command::new("xcode-select")
        .arg("--print-path")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default();
    if !developer_dir.ends_with("CommandLineTools") {
        return command;
    }
    command.args(["--build-system", "native"]);
    let developer_dir = Path::new(&developer_dir);
    let has_swiftui_macros = developer_dir
        .join("usr/lib/swift/host/plugins/libSwiftUIMacros.dylib")
        .exists();
    if std::env::var_os("SDKROOT").is_none()
        && !has_swiftui_macros
        && let Some(sdk) = newest_sdk_up_to(&developer_dir.join("SDKs"), 26)
    {
        command.env("SDKROOT", sdk);
    }
    if subcommand == "test" {
        // With a non-default SDK, SwiftPM no longer finds the Testing framework by itself.
        let frameworks = developer_dir.join("Library/Developer/Frameworks");
        let libraries = developer_dir.join("Library/Developer/usr/lib");
        command
            .arg("-Xswiftc")
            .arg("-F")
            .arg("-Xswiftc")
            .arg(&frameworks);
        for (flag, path) in [
            ("-F", &frameworks),
            ("-rpath", &frameworks),
            ("-rpath", &libraries),
        ] {
            command.arg("-Xlinker").arg(flag).arg("-Xlinker").arg(path);
        }
    }
    command
}

/// The newest `MacOSX<major>[.<minor>].sdk` in `dir` with a major version of at most `max_major`.
fn newest_sdk_up_to(dir: &Path, max_major: u32) -> Option<PathBuf> {
    let mut best: Option<((u32, u32), PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(version) = name
            .strip_prefix("MacOSX")
            .and_then(|rest| rest.strip_suffix(".sdk"))
        else {
            continue;
        };
        let mut parts = version.split('.').map(|part| part.parse::<u32>().ok());
        let (Some(Some(major)), minor) = (parts.next(), parts.next().flatten().unwrap_or(0)) else {
            continue;
        };
        if major <= max_major
            && best
                .as_ref()
                .is_none_or(|(found, _)| (major, minor) > *found)
        {
            best = Some(((major, minor), entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// Builds the askpass helper for the app's architectures. Returns its path.
fn build_askpass(options: Options) -> Result<PathBuf> {
    let profile = if options.release { "release" } else { "debug" };
    let targets: Vec<Option<&str>> = if options.universal {
        vec![Some("aarch64-apple-darwin"), Some("x86_64-apple-darwin")]
    } else {
        vec![None]
    };
    let mut binaries = Vec::new();
    for target in &targets {
        let mut build = cargo();
        build
            .args(["build", "--package", "branchosaurus-core", "--bin", ASKPASS])
            .env("MACOSX_DEPLOYMENT_TARGET", MACOS_DEPLOYMENT_TARGET);
        if options.release {
            build.arg("--release");
        }
        if let Some(target) = target {
            build.args(["--target", target]);
        }
        run(&mut build)?;
        let mut binary = workspace_root().join("target");
        if let Some(target) = target {
            binary.push(target);
        }
        binaries.push(binary.join(profile).join(ASKPASS));
    }
    let output = output_dir().join("bin").join(ASKPASS);
    fs::create_dir_all(output.parent().expect("bin dir"))?;
    if binaries.len() == 1 {
        fs::copy(&binaries[0], &output)?;
    } else {
        run(Command::new("lipo")
            .arg("-create")
            .args(&binaries)
            .arg("-output")
            .arg(&output))?;
    }
    Ok(output)
}

/// The third-party licenses that must ship with the app, as Markdown.
fn acknowledgements(options: Options) -> Result<Vec<u8>> {
    let mut command = cargo();
    command.args([
        "run",
        "--quiet",
        "--package",
        "branchosaurus-core",
        "--example",
        "acknowledgements",
    ]);
    if options.release {
        command.arg("--release");
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "printing the acknowledgements failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output.stdout)
}

fn macos_test(options: Options) -> Result {
    swift_bindings(options)?;
    run(&mut swift("test"))
}

fn macos_app(options: Options) -> Result {
    if !cfg!(target_os = "macos") {
        return Err("the macOS app can only be built on macOS".into());
    }
    swift_bindings(options)?;
    let askpass = build_askpass(options)?;

    let configuration = if options.release { "release" } else { "debug" };
    let triples: Vec<Option<&str>> = if options.universal {
        vec![
            Some("arm64-apple-macosx14.0"),
            Some("x86_64-apple-macosx14.0"),
        ]
    } else {
        vec![None]
    };
    let mut binaries = Vec::new();
    for triple in &triples {
        let mut build = swift_build();
        build.args(["--configuration", configuration, "--product", APP_NAME]);
        if let Some(triple) = triple {
            build.args(["--triple", triple]);
        }
        run(&mut build)?;
        let mut show_bin_path = swift_build();
        show_bin_path.args(["--configuration", configuration, "--show-bin-path"]);
        if let Some(triple) = triple {
            show_bin_path.args(["--triple", triple]);
        }
        let output = show_bin_path.output()?;
        let bin_dir = String::from_utf8(output.stdout)?.trim().to_owned();
        binaries.push(PathBuf::from(bin_dir).join(APP_NAME));
    }

    let app = output_dir().join(format!("{APP_NAME}.app"));
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    let contents = app.join("Contents");
    let executable_dir = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&executable_dir)?;
    fs::create_dir_all(&resources)?;
    let executable = executable_dir.join(APP_NAME);
    if binaries.len() == 1 {
        fs::copy(&binaries[0], &executable)?;
    } else {
        run(Command::new("lipo")
            .arg("-create")
            .args(&binaries)
            .arg("-output")
            .arg(&executable))?;
    }
    fs::copy(&askpass, executable_dir.join(ASKPASS))?;

    let app_resources = macos_dir().join("Resources");
    let version = env!("CARGO_PKG_VERSION");
    let info_plist = fs::read_to_string(app_resources.join("Info.plist"))?
        .replace("$(VERSION)", version)
        .replace("$(MACOS_DEPLOYMENT_TARGET)", MACOS_DEPLOYMENT_TARGET);
    fs::write(contents.join("Info.plist"), info_plist)?;
    let icon = app_resources.join("AppIcon.icns");
    if icon.exists() {
        fs::copy(&icon, resources.join("AppIcon.icns"))?;
    }
    fs::write(
        resources.join("Acknowledgements.md"),
        acknowledgements(options)?,
    )?;

    // Ad-hoc signature: enough to run locally and to keep the hardened-runtime tooling happy.
    // Nested executables are signed before the bundle that contains them.
    for target in [executable_dir.join(ASKPASS), app.clone()] {
        run(Command::new("codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(&target))?;
    }
    println!("Built {}", app.display());
    Ok(())
}
