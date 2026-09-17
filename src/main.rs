use std::collections::HashMap;
use std::io::Write as _;

use haxedecomp::abc::parse_abc;
use haxedecomp::decompile::decompile_abc;
use haxedecomp::dump::dump_abc;
use haxedecomp::swf::{iter_abc_tags, parse_swf, tag_name};

fn print_err(msg: &str) {
    let mut e = std::io::stderr().lock();
    let _ = writeln!(e, "{}", msg);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("Usage: haxedecomp <file.swf|file.abc> [--code] [--decompile]");
        std::process::exit(1);
    }
    let path = args[1].clone();
    let show_code = args.iter().any(|a| a == "--code");
    let decompile_mode = args.iter().any(|a| a == "--decompile");

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            print_err(&format!("error: cannot read {}: {}", path, e));
            std::process::exit(1);
        }
    };
    let sig = &data[..data.len().min(3)];

    if sig == b"FWS" || sig == b"CWS" || sig == b"ZWS" {
        let (header, tags) = match parse_swf(&data) {
            Ok(v) => v,
            Err(e) => {
                print_err(&format!("error: {}", e));
                std::process::exit(1);
            }
        };
        if !decompile_mode {
            println!(
                "SWF v{} {} {:.0}x{:.0} fps={} frames={} compressed={}",
                header.version,
                header.signature,
                header.frame_size.left as f64 / 20.0,
                header.frame_size.top as f64 / 20.0,
                haxedecomp::haxe_out::py_repr_f64(header.fps),
                header.frame_count,
                if header.compressed { "True" } else { "False" }
            );
            println!("Tags: {}", tags.len());
        }
        let mut abc_count = 0;
        for (info, abc_data) in iter_abc_tags(&tags) {
            abc_count += 1;
            let label = match &info {
                Some((id, frame)) => format!("tag-{} frame={:?} id={}", abc_count, frame, id),
                None => format!("tag-{}", abc_count),
            };
            match parse_abc(abc_data) {
                Ok(abc) => {
                    if decompile_mode {
                        print!("{}", decompile_abc(&abc, Some(&header.symbols)));
                    } else {
                        println!("{}", dump_abc(&abc, Some(&label), show_code));
                    }
                }
                Err(e) => {
                    println!("Failed to parse ABC in {}: {}", label, e);
                }
            }
        }
        if abc_count == 0 && !decompile_mode {
            println!("No AVM2 ABC tags found.");
            for t in &tags {
                if t.tid != 0 && t.tid != 1 {
                    println!("  tag 0x{:02X} ({}) len={}", t.tid, tag_name(t.tid), t.data.len());
                }
            }
        }
    } else if data.len() >= 4
        && (data[0] == 0x10 && data[1] == 0x00 && data[2] == 0x2E && data[3] == 0x00
            || data[0] == 0x00 && data[1] == 0x2E && data[2] == 0x00 && data[3] == 0x10)
    {
        match parse_abc(&data) {
            Ok(abc) => {
                let label = std::path::Path::new(&path)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                if decompile_mode {
                    let syms: HashMap<String, u16> = HashMap::new();
                    print!("{}", decompile_abc(&abc, Some(&syms)));
                } else {
                    print!("{}", dump_abc(&abc, Some(&label), show_code));
                }
            }
            Err(e) => {
                print_err(&format!("error: {}", e));
                std::process::exit(1);
            }
        }
    } else {
        println!(
            "Unknown file signature: {:?}",
            String::from_utf8_lossy(sig)
        );
        std::process::exit(1);
    }
}
