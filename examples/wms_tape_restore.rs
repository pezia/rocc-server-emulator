//! Assisted operator IPL for the recovered WMS system-save tape layout.
//! Run only against a blank base image and a new destination overlay.

use rocc_emulator::{machine::Machine, tape::TapeMount};
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    time::Instant,
};

fn dma_record(m: &mut Machine, address: u16, count: u16, expected: u16) -> Result<(), String> {
    for (op, pulse, value) in [(2, 0, 0), (4, 0, address), (6, 1, count)] {
        m.bus
            .io(&mut m.cpu.memory, 0o20, op, pulse, value)
            .map_err(|f| f.reason().to_owned())?;
    }
    m.step();
    if let Some(f) = &m.fault {
        return Err(f.reason().into());
    }
    let end = m
        .bus
        .io(&mut m.cpu.memory, 0o20, 3, 0, 0)
        .map_err(|f| f.reason().to_owned())?
        .value;
    if end != Some(expected) {
        return Err("unexpected WMS boot tape record length".into());
    }
    Ok(())
}

fn prepare_wms_load(m: &mut Machine) -> Result<(), String> {
    if !m.config().extended()
        || m.steps != 0
        || m.memory_status().enabled
        || !m
            .tape_status()
            .is_some_and(|t| t.mode == "read_only" && t.position == 0)
    {
        return Err("WMS tape IPL requires a fresh extended machine, read-only reel at BOT, and no retained RAM".into());
    }
    // The operator memory-clear operation establishes zero in unused physical banks.
    let cells = m.cpu.memory.iter_physical().count();
    for n in 0..cells {
        m.cpu
            .memory
            .initialize_physical(n, 0)
            .map_err(|f| f.reason().to_owned())?;
    }
    m.cpu
        .memory
        .set_mapping(rocc_emulator::memory::Mapping::SelectedBank(0))
        .map_err(|f| f.reason().to_owned())?;
    m.cpu.pc = 127;
    dma_record(m, 0x0847, 0x7f80, 0x08c7)?;
    for (i, w) in [0xa72e, 1, 0x8301, 0x0040].into_iter().enumerate() {
        if m.cpu
            .memory
            .read(0x0847 + i)
            .map_err(|f| f.reason().to_owned())?
            != w
        {
            return Err("not the recovered WMS system-save bootstrap".into());
        }
    }
    dma_record(m, 0, 0x7000, 0x1000)?;
    if m.cpu
        .memory
        .read(0x0867)
        .map_err(|f| f.reason().to_owned())?
        != 0x6750
    {
        return Err("unrecognized WMS native tape-load entry".into());
    }
    // Enter the original busy-poll loop after its first RAM page has arrived.
    m.cpu.pc = 0x0867;
    m.cpu.ac = [0; 4];
    m.cpu.carry = 0;
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if !(3..=4).contains(&args.len()) {
        return Err("usage: wms_tape_restore BLANK_IMAGE TAPE NEW_OVERLAY [NEW_REPORT]".into());
    }
    let overlay =
        rocc_emulator::artifact::check_destination(&args[2]).map_err(|e| e.to_string())?;
    let report = args
        .get(3)
        .map(|p| rocc_emulator::artifact::check_destination(p))
        .transpose()
        .map_err(|e| e.to_string())?;
    if report.as_ref() == Some(&overlay) {
        return Err("report and overlay require separate paths".into());
    }
    let image = File::open(&args[0]).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    (&image)
        .take(512 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.iter().any(|&b| b != 0) {
        return Err("destination base image must be blank (all zero); choose a separate restoration destination".into());
    }
    let start = Instant::now();
    let mut m = Machine::new(raw, true, 16, true, Some(1))?;
    m.attach_overlay(&overlay, &image)?;
    m.attach_tape(
        TapeMount::ReadOnly(args[1].clone()),
        &args[0],
        &overlay,
        report.as_deref(),
    )?;
    prepare_wms_load(&mut m)?;
    let budget = 150_000_000;
    while m.steps < budget && m.reason.is_none() {
        m.step();
    }
    m.finish_tape()?;
    if let Some(report) = report {
        let metadata = rocc_emulator::report::RunMetadata {
            input: Some(fs::canonicalize(&args[0]).map_err(|e| e.to_string())?),
            overlay_path: Some(overlay.clone()),
            elapsed_seconds: start.elapsed().as_secs_f64(),
            step_budget: Some(budget),
            ..Default::default()
        };
        rocc_emulator::artifact::publish_new(&report, |f| {
            rocc_emulator::report::write_report(f, &m, &metadata)
        })
        .map_err(|e| e.to_string())?;
    }
    let tape = m.tape_status().ok_or("tape disappeared")?;
    println!("Native load stopped after {} instructions: {}; {} tape reads, {} disk writes.\nDestination overlay: {}\nVerify restored contents, then cold-boot the destination. Retained RAM was not loaded or saved.",m.steps,m.summary().reason,tape.reads,m.bus.disk.writes,overlay.display());
    if let Some(f) = &m.fault {
        return Err(format!("native loader stopped: {}", f.reason()));
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("wms_tape_restore: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use rocc_emulator::{machine::Machine, tape::TapeMount};
    use std::{fs, fs::File};
    #[test]
    fn assisted_ipl_reads_native_boot_records_and_clears_unused_banks() {
        let dir = std::env::temp_dir().join(format!("rocc-tape-ipl-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("blank");
        fs::write(&image, vec![0; 6144]).unwrap();
        let mut raw = Vec::new();
        let mut header = vec![0; 256];
        for (i, w) in [0xa72eu16, 1, 0x8301, 0x0040].into_iter().enumerate() {
            header[2 * i..2 * i + 2].copy_from_slice(&w.to_be_bytes());
        }
        let mut page = vec![0; 8192];
        page[0x0867 * 2..0x0867 * 2 + 2].copy_from_slice(&0x6750u16.to_be_bytes());
        page[0x0869 * 2..0x0869 * 2 + 2].copy_from_slice(&0x663fu16.to_be_bytes());
        page[600..602].copy_from_slice(&0x1234u16.to_be_bytes());
        for data in [&header, &page] {
            raw.extend_from_slice(&(data.len() as u32).to_le_bytes());
            raw.extend_from_slice(data);
            raw.extend_from_slice(&(data.len() as u32).to_le_bytes());
        }
        let tape = dir.join("reel");
        fs::write(&tape, raw).unwrap();
        let mut m = Machine::new(vec![0; 6144], true, 16, false, None).unwrap();
        let f = File::open(&image).unwrap();
        let overlay = dir.join("overlay");
        m.attach_overlay(&overlay, &f).unwrap();
        m.attach_tape(TapeMount::ReadOnly(tape), &image, &overlay, None)
            .unwrap();
        super::prepare_wms_load(&mut m).unwrap();
        assert_eq!(m.cpu.pc, 0x0867);
        assert_eq!(m.cpu.memory.read(300).unwrap(), 0x1234);
        assert_eq!(m.tape_status().unwrap().reads, 2);
        m.cpu
            .memory
            .set_mapping(rocc_emulator::memory::Mapping::SelectedBank(105))
            .unwrap();
        assert_eq!(m.cpu.memory.read(0xf000).unwrap(), 0);
        m.step();
        m.step();
        assert_eq!(m.reason.as_deref(), Some("halt"));
        drop(m);
        fs::remove_dir_all(dir).unwrap();
    }
}
