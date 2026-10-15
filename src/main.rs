use rocc_emulator::{
    cpu::Cpu,
    devices::Bus,
    machine::{self, Machine},
};
use serde_json::{json, Value};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

fn main() {
    if let Err(e) = run() {
        eprintln!("rocc-emulator: {e}");
        std::process::exit(1);
    }
}
fn help() {
    println!(
        "Native ROCC emulator (experimental inferred hardware)\n\
Usage: rocc-emulator boot IMAGE [OPTIONS]\n\
  --keymap PATH             JSON host-key to raw ROCC-byte bindings (interactive)\n\
  --interactive             Live terminal (Ctrl-] or Ctrl-C exits)\n\
  --steps N                 Instructions: default 1000000, unlimited live\n\
  --profile loader|extended Default loader, extended for live\n\
  --terminal none|experimental\n\
  --connect-after N         Connect terminal at instruction N (live:26000000)\n\
  --address-bits 15|16      Extended requires16\n\
  --expected-sha256 HASH    Reject a different image\n\
  --overlay PATH            Persistent disk writes (default: IMAGE.overlay)\n\
  --report NEWPATH          Create JSON report including changed disk blocks\n\
  --key-at STEP:HEX         Inject byte before zero-based STEP (repeatable)\n\
  cpu-cases                JSON instruction fixtures on stdin for differential tests"
    );
}
fn number(s: &str, label: &str) -> Result<u64, String> {
    s.parse().map_err(|_| format!("invalid {label}: {s}"))
}
fn run() -> Result<(), String> {
    let mut args = env::args().skip(1).peekable();
    if matches!(
        args.peek().map(String::as_str),
        None | Some("--help" | "-h")
    ) {
        help();
        return Ok(());
    }
    if args.peek().map(String::as_str) == Some("cpu-cases") {
        args.next();
        if args.next().is_some() {
            return Err("cpu-cases takes no arguments".into());
        }
        return cpu_cases();
    }
    if args.peek().map(String::as_str) == Some("boot") {
        args.next();
    }
    let mut image = None;
    let mut report = None;
    let mut overlay = None;
    let mut profile = None;
    let mut terminal = None;
    let mut interactive = false;
    let mut keymap = None;
    let mut steps = None;
    let mut connect = None;
    let mut bits = None;
    let mut expected = None;
    let mut keys = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--interactive" {
            interactive = true;
            continue;
        }
        if arg == "--help" || arg == "-h" {
            help();
            return Ok(());
        }
        if !arg.starts_with('-') {
            if image.replace(PathBuf::from(arg)).is_some() {
                return Err("only one image is allowed".into());
            }
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--keymap" => keymap = Some(value),
            "--steps" => steps = Some(number(&value, "step budget")?),
            "--connect-after" => connect = Some(number(&value, "connection time")?),
            "--address-bits" => bits = Some(number(&value, "address bits")?),
            "--profile" => profile = Some(value),
            "--terminal" => terminal = Some(value),
            "--expected-sha256" => expected = Some(value),
            "--report" => report = Some(PathBuf::from(value)),
            "--overlay" => overlay = Some(PathBuf::from(value)),
            "--key-at" => {
                let (step, byte) = value
                    .split_once(':')
                    .ok_or("key schedule must be STEP:HEX")?;
                if byte.len() != 2 {
                    return Err("key must contain exactly two hex digits".into());
                }
                keys.push((
                    number(step, "key step")?,
                    u8::from_str_radix(byte, 16).map_err(|_| "invalid key byte")?,
                ));
            }
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    let bindings = if let Some(path) = keymap {
        if !interactive {
            return Err("--keymap requires --interactive".into());
        }
        rocc_emulator::interactive::parse_keymap(
            &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
        )?
    } else {
        Default::default()
    };
    let extended =
        match profile
            .as_deref()
            .unwrap_or(if interactive { "extended" } else { "loader" })
        {
            "extended" => true,
            "loader" => false,
            _ => return Err("unknown profile".into()),
        };
    let experimental =
        match terminal
            .as_deref()
            .unwrap_or(if interactive { "experimental" } else { "none" })
        {
            "experimental" => true,
            "none" => false,
            _ => return Err("unknown terminal profile".into()),
        };
    if interactive && (!extended || !experimental) {
        return Err("interactive requires extended experimental terminal".into());
    }
    if experimental && !extended {
        return Err("terminal requires extended profile".into());
    }
    let bits = bits.unwrap_or(if extended { 16 } else { 15 });
    if ![15, 16].contains(&bits) || (extended && bits != 16) {
        return Err("invalid address width for profile".into());
    }
    if !interactive && steps.is_none() {
        steps = Some(1_000_000);
    }
    if steps.is_some_and(|n| n == 0 || n > 100_000_000) {
        return Err("steps must be1..100000000".into());
    }
    if interactive && connect.is_none() {
        connect = Some(26_000_000);
    }
    if let Some(c) = connect {
        if !experimental || c == 0 || steps.is_some_and(|n| c > n) {
            return Err("connection time must fit the budget and use experimental terminal".into());
        }
    }
    if !keys.is_empty() && (!experimental || interactive) {
        return Err("scheduled keys require batch experimental terminal".into());
    }
    if keys.iter().any(|(s, _)| steps.is_some_and(|n| *s >= n)) {
        return Err("scheduled key outside step budget".into());
    }
    keys.sort_by_key(|(s, _)| *s);
    if interactive && unsafe { libc::isatty(0) != 1 || libc::isatty(1) != 1 } {
        return Err("interactive mode requires a terminal on stdin and stdout".into());
    }
    let image = image.ok_or("image path is required")?;
    let overlay = overlay.unwrap_or_else(|| {
        let mut name = image.as_os_str().to_os_string();
        name.push(".overlay");
        PathBuf::from(name)
    });
    let destination = report.as_deref().map(check_destination).transpose()?;
    let mut raw = Vec::new();
    let image_file = File::open(&image).map_err(|e| e.to_string())?;
    (&image_file)
        .take(512 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut m = Machine::new(raw, extended, bits as u8, experimental, connect)?;
    if expected.as_ref().is_some_and(|v| v != &m.image_sha256) {
        return Err("image SHA256 differs from expected value".into());
    }
    m.attach_overlay(&overlay, &image_file)?;
    let overlay_path = fs::canonicalize(&overlay).map_err(|e| e.to_string())?;
    if destination.as_ref() == Some(&overlay_path) {
        return Err("report and overlay must use separate paths".into());
    }
    let interactive_report = if interactive {
        Some(rocc_emulator::interactive::run(&mut m, steps, bindings)?)
    } else {
        let mut key_index = 0;
        while m.steps < steps.unwrap() && m.reason.is_none() {
            while key_index < keys.len() && keys[key_index].0 == m.steps {
                m.bus
                    .terminal
                    .send_key(keys[key_index].1)
                    .map_err(|f| f.reason)?;
                key_index += 1;
            }
            m.step();
        }
        None
    };
    let elapsed = started.elapsed().as_secs_f64();
    let mut data = m.report();
    data["elapsed_seconds"] = json!(elapsed);
    data["input"] = json!(fs::canonicalize(&image).map_err(|e| e.to_string())?);
    data["overlay_path"] = json!(overlay_path);
    if let Some(v) = interactive_report {
        data["interactive"] = v;
    }
    if let Some(dest) = destination {
        publish(&dest, &data)?;
        println!("Report: {}", dest.display());
    }
    println!(
        "ROCC Rust: {} at RAM word {} after {} instructions ({elapsed:.3}s)",
        data["reason"].as_str().unwrap(),
        m.cpu.pc,
        m.steps
    );
    println!(
        "Disk: {} reads, {} writes to persistent overlay ({} blocks). Original image unchanged.",
        m.bus.disk.reads,
        m.bus.disk.writes,
        m.bus.disk.overlay.len()
    );
    println!("Overlay: {}", overlay.display());
    if m.bus.terminal.total_display_bytes > 0 {
        for line in data["display"]["screen"]["lines"].as_array().unwrap() {
            println!("{}", line.as_str().unwrap());
        }
    }
    if m.fault.is_some() {
        return Err(format!("guest stopped: {}", m.reason.unwrap()));
    }
    Ok(())
}

fn check_destination(path: &Path) -> Result<PathBuf, String> {
    if fs::symlink_metadata(path).is_ok() {
        return Err("report already exists; choose a new path".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|e| format!("report parent: {e}"))?;
    let dest = parent.join(path.file_name().ok_or("report needs a filename")?);
    Ok(dest)
}
fn publish(dest: &Path, data: &Value) -> Result<(), String> {
    let temporary = dest
        .parent()
        .unwrap()
        .join(format!(".rocc-rust-report-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        serde_json::to_writer_pretty(&mut file, data).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::hard_link(&temporary, dest).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let cleanup = fs::remove_file(&temporary).map_err(|e| e.to_string());
    result.and(cleanup)
}

fn uint(v: &Value, key: &str, max: u64) -> Result<u64, String> {
    v[key]
        .as_u64()
        .filter(|&n| n <= max)
        .ok_or_else(|| format!("invalid fixture {key}"))
}
fn cpu_cases() -> Result<(), String> {
    let mut raw = String::new();
    io::stdin()
        .take(32 * 1024 * 1024 + 1)
        .read_to_string(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > 32 * 1024 * 1024 {
        return Err("fixture input exceeds32MiB".into());
    }
    let values: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let cases = values.as_array().ok_or("fixtures must be an array")?;
    let mut out = Vec::new();
    for c in cases {
        let extended = c["extended"].as_bool().ok_or("invalid extended flag")?;
        let bits = uint(c, "address_bits", 16)? as u8;
        if ![15, 16].contains(&bits) || (extended && bits != 16) {
            return Err("invalid fixture bits".into());
        }
        let mut cpu = Cpu::new(bits, extended);
        cpu.pc = uint(c, "pc", cpu.memory.mask as u64)? as u16;
        cpu.carry = uint(c, "carry", 1)? as u16;
        let ac = c["ac"]
            .as_array()
            .filter(|a| a.len() == 4)
            .ok_or("invalid fixture accumulators")?;
        for (i, a) in ac.iter().enumerate() {
            cpu.ac[i] = a
                .as_u64()
                .filter(|&n| n <= 65535)
                .ok_or("invalid accumulator")? as u16;
        }
        if c.get("bank").is_some() {
            cpu.memory.bank = uint(c, "bank", (cpu.memory.banks - 1) as u64)? as usize;
        }
        cpu.global_only = c["global_only"].as_bool().unwrap_or(false);
        for entry in c["memory"].as_array().ok_or("invalid fixture memory")? {
            let a = entry
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or("invalid memory entry")?;
            let addr = a[0]
                .as_u64()
                .filter(|&n| n < (cpu.memory.storage.len() as u64))
                .ok_or("invalid physical address")? as usize;
            let val = a[1]
                .as_u64()
                .filter(|&n| n <= 65535)
                .ok_or("invalid word")? as u16;
            cpu.memory.storage[addr] = Some(val);
        }
        let steps = uint(c, "steps", 100_000)?;
        let mut bus = Bus::new(vec![0; 6144], extended, false, None);
        let mut reason = "step_limit".to_string();
        for _ in 0..steps {
            match cpu.step(&mut bus) {
                Ok(i) => {
                    if i.halt {
                        reason = "halt".into();
                        break;
                    }
                }
                Err(f) => {
                    reason = f.reason;
                    break;
                }
            }
        }
        let memory: Vec<_> = cpu
            .memory
            .storage
            .iter()
            .enumerate()
            .filter_map(|(a, v)| v.map(|w| json!([a, w])))
            .collect();
        out.push(
            json!({"state":machine::cpu_state(&cpu,extended),"reason":reason,"memory":memory}),
        );
    }
    serde_json::to_writer(io::stdout().lock(), &out).map_err(|e| e.to_string())?;
    Ok(())
}
