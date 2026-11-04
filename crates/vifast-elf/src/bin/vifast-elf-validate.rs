use std::env;
use std::fs;
use std::process;

fn parse_u64(value: &str) -> u64 {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).unwrap_or_else(|_| {
            eprintln!("invalid hex value: {value}");
            process::exit(2);
        })
    } else {
        value.parse().unwrap_or_else(|_| {
            eprintln!("invalid number: {value}");
            process::exit(2);
        })
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        eprintln!("usage: {} <elf> <machine> <user_start> <user_end>", args[0]);
        process::exit(2);
    }
    let machine: u16 = args[2].parse().unwrap_or_else(|_| {
        eprintln!("invalid machine: {}", args[2]);
        process::exit(2);
    });
    let user_start = parse_u64(&args[3]);
    let user_end = parse_u64(&args[4]);
    let bytes = fs::read(&args[1]).unwrap_or_else(|error| {
        eprintln!("cannot read {}: {error}", args[1]);
        process::exit(2);
    });
    match vifast_elf::parse(&bytes, machine, user_start, user_end) {
        Ok(image) => {
            println!(
                "ELF OK machine={} entry={:#x} segments={}",
                image.machine,
                image.entry,
                image.segment_count()
            );
            for segment in image.segments() {
                println!(
                    "  vaddr={:#x} filesz={:#x} memsz={:#x} flags={:#x}",
                    segment.vaddr, segment.file_size, segment.memory_size, segment.flags
                );
            }
        }
        Err(error) => {
            eprintln!("ELF INVALID: {error:?}");
            process::exit(1);
        }
    }
}
