use std::collections::HashMap;
use crate::bitio::{BitReader, ByteReader};
use crate::inflate::{decompress_zlib, lzma_decompress_zws};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rect {
    pub nbits: u32,
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone)]
pub struct SwfHeader {
    pub signature: String,
    pub version: u8,
    pub file_length: u32,
    pub frame_size: Rect,
    pub fps: f64,
    pub frame_count: u16,
    pub compressed: bool,
    pub parse_warnings: Vec<String>,
    pub symbols: HashMap<String, u16>,
    pub entry_class: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub tid: u16,
    pub extended: bool,
    pub data: Vec<u8>,
}

pub const TAG_END: u16 = 0x00;
pub const TAG_SHOWFRAME: u16 = 0x01;
pub const TAG_DEFINESHAPE: u16 = 0x02;
pub const TAG_DEFINEBITSJPEG: u16 = 0x06;
pub const TAG_JPEGTABLES: u16 = 0x08;
pub const TAG_SETBGCOLOR: u16 = 0x09;
pub const TAG_DEFINEFONT: u16 = 0x0A;
pub const TAG_DEFINETEXT: u16 = 0x0B;
pub const TAG_DOACTION: u16 = 0x0C;
pub const TAG_FONTINFO: u16 = 0x0D;
pub const TAG_SOUND: u16 = 0x0E;
pub const TAG_STARTSOUND: u16 = 0x0F;
pub const TAG_DEFINELOSSLESS: u16 = 0x14;
pub const TAG_DEFINEBITSJPEG2: u16 = 0x15;
pub const TAG_DEFINESHAPE2: u16 = 0x16;
pub const TAG_PROTECT: u16 = 0x18;
pub const TAG_PLACEOBJECT2: u16 = 0x1A;
pub const TAG_REMOVEOBJECT2: u16 = 0x1C;
pub const TAG_DEFINESHAPE3: u16 = 0x20;
pub const TAG_DEFINETEXT2: u16 = 0x21;
pub const TAG_DEFINEBUTTON2: u16 = 0x22;
pub const TAG_DEFINEBITSJPEG3: u16 = 0x23;
pub const TAG_DEFINELOSSLESS2: u16 = 0x24;
pub const TAG_EDITTEXT: u16 = 0x25;
pub const TAG_DEFINESPRITE: u16 = 0x27;
pub const TAG_PRODUCTINFO: u16 = 0x29;
pub const TAG_FRAMELABEL: u16 = 0x2B;
pub const TAG_SOUNDSTREAMHEAD2: u16 = 0x2D;
pub const TAG_DEFINEMORPHSHAPE: u16 = 0x2E;
pub const TAG_DEFINEFONT2: u16 = 0x30;
pub const TAG_EXPORT: u16 = 0x38;
pub const TAG_IMPORT: u16 = 0x39;
pub const TAG_DOINITACTION: u16 = 0x3B;
pub const TAG_VIDEOSTREAM: u16 = 0x3C;
pub const TAG_VIDEOFRAME: u16 = 0x3D;
pub const TAG_FONTINFO2: u16 = 0x3E;
pub const TAG_DEBUGID: u16 = 0x3F;
pub const TAG_ENABLEDEBUGGER2: u16 = 0x40;
pub const TAG_SCRIPTLIMITS: u16 = 0x41;
pub const TAG_FILESATTRIBUTES: u16 = 0x45;
pub const TAG_PLACEOBJECT3: u16 = 0x46;
pub const TAG_IMPORT2: u16 = 0x47;
pub const TAG_DOABCDEFINE: u16 = 0x48;
pub const TAG_FONTALIGNZONES: u16 = 0x49;
pub const TAG_CSMSETTINGS: u16 = 0x4A;
pub const TAG_DEFINEFONT3: u16 = 0x4B;
pub const TAG_SYMBOLCLASS: u16 = 0x4C;
pub const TAG_METADATA: u16 = 0x4D;
pub const TAG_SCALE9: u16 = 0x4E;
pub const TAG_DOABC: u16 = 0x52;
pub const TAG_DEFINESHAPE4: u16 = 0x53;
pub const TAG_DEFINEMORPHSHAPE2: u16 = 0x54;
pub const TAG_DEFINESCENES: u16 = 0x56;
pub const TAG_BINARYDATA: u16 = 0x57;
pub const TAG_FONTNAME: u16 = 0x58;
pub const TAG_DEFINEBITSJPEG4: u16 = 0x5A;
pub const TAG_DEFINEFONT4: u16 = 0x5B;

pub fn tag_name(tid: u16) -> String {
    match tid {
        TAG_END => "End".to_string(),
        TAG_SHOWFRAME => "ShowFrame".to_string(),
        TAG_DEFINESHAPE => "DefineShape".to_string(),
        TAG_DEFINEBITSJPEG => "DefineBitsJPEG".to_string(),
        TAG_JPEGTABLES => "JPEGTables".to_string(),
        TAG_SETBGCOLOR => "SetBackgroundColor".to_string(),
        TAG_DEFINEFONT => "DefineFont".to_string(),
        TAG_DEFINETEXT => "DefineText".to_string(),
        TAG_DOACTION => "DoAction".to_string(),
        TAG_FONTINFO => "FontInfo".to_string(),
        TAG_SOUND => "Sound".to_string(),
        TAG_STARTSOUND => "StartSound".to_string(),
        TAG_DEFINELOSSLESS => "DefineLossless".to_string(),
        TAG_DEFINEBITSJPEG2 => "DefineBitsJPEG2".to_string(),
        TAG_DEFINESHAPE2 => "DefineShape2".to_string(),
        TAG_PROTECT => "Protect".to_string(),
        TAG_PLACEOBJECT2 => "PlaceObject2".to_string(),
        TAG_REMOVEOBJECT2 => "RemoveObject2".to_string(),
        TAG_DEFINESHAPE3 => "DefineShape3".to_string(),
        TAG_DEFINETEXT2 => "DefineText2".to_string(),
        TAG_DEFINEBUTTON2 => "DefineButton2".to_string(),
        TAG_DEFINEBITSJPEG3 => "DefineBitsJPEG3".to_string(),
        TAG_DEFINELOSSLESS2 => "DefineLossless2".to_string(),
        TAG_EDITTEXT => "EditText".to_string(),
        TAG_DEFINESPRITE => "DefineSprite".to_string(),
        TAG_PRODUCTINFO => "ProductInfo".to_string(),
        TAG_FRAMELABEL => "FrameLabel".to_string(),
        TAG_SOUNDSTREAMHEAD2 => "SoundStreamHead2".to_string(),
        TAG_DEFINEMORPHSHAPE => "DefineMorphShape".to_string(),
        TAG_DEFINEFONT2 => "DefineFont2".to_string(),
        TAG_EXPORT => "ExportAssets".to_string(),
        TAG_IMPORT => "ImportAssets".to_string(),
        TAG_DOINITACTION => "DoInitAction".to_string(),
        TAG_VIDEOSTREAM => "VideoStream".to_string(),
        TAG_VIDEOFRAME => "VideoFrame".to_string(),
        TAG_FONTINFO2 => "FontInfo2".to_string(),
        TAG_DEBUGID => "DebugID".to_string(),
        TAG_ENABLEDEBUGGER2 => "EnableDebugger2".to_string(),
        TAG_SCRIPTLIMITS => "ScriptLimits".to_string(),
        TAG_FILESATTRIBUTES => "FileAttributes".to_string(),
        TAG_PLACEOBJECT3 => "PlaceObject3".to_string(),
        TAG_IMPORT2 => "ImportAssets2".to_string(),
        TAG_DOABCDEFINE => "DoABC(main)".to_string(),
        TAG_FONTALIGNZONES => "FontAlignZones".to_string(),
        TAG_CSMSETTINGS => "CSMSettings".to_string(),
        TAG_DEFINEFONT3 => "DefineFont3".to_string(),
        TAG_SYMBOLCLASS => "SymbolClass".to_string(),
        TAG_METADATA => "Metadata".to_string(),
        TAG_SCALE9 => "Scale9".to_string(),
        TAG_DOABC => "DoABC".to_string(),
        TAG_DEFINESHAPE4 => "DefineShape4".to_string(),
        TAG_DEFINEMORPHSHAPE2 => "DefineMorphShape2".to_string(),
        TAG_DEFINESCENES => "DefineSceneAndFrameLabelData".to_string(),
        TAG_BINARYDATA => "DefineBinaryData".to_string(),
        TAG_FONTNAME => "FontName".to_string(),
        TAG_DEFINEBITSJPEG4 => "DefineBitsJPEG4".to_string(),
        TAG_DEFINEFONT4 => "DefineFont4".to_string(),
        _ => format!("Unknown_0x{:02X}", tid),
    }
}

pub fn read_rect(br: &mut ByteReader) -> Result<Rect, String> {
    let mut bits = BitReader::new(br.data, br.pos);
    let nbits = bits.read_bits(5);
    let left = bits.read_sbits(nbits);
    let right = bits.read_sbits(nbits);
    let top = bits.read_sbits(nbits);
    let bottom = bits.read_sbits(nbits);
    br.seek(bits.pos);
    Ok(Rect {
        nbits,
        left,
        right,
        top,
        bottom,
    })
}

pub fn parse_swf(data: &[u8]) -> Result<(SwfHeader, Vec<Tag>), String> {
    if data.len() < 8 {
        return Err("SWF too short".to_string());
    }

    let sig_bytes = &data[0..3];
    let sig = std::str::from_utf8(sig_bytes).map_err(|_| "Invalid SWF signature utf-8")?;
    if sig != "FWS" && sig != "CWS" && sig != "ZWS" {
        return Err(format!("Not a SWF file (signature={:?})", sig));
    }

    let version = data[3];
    let file_length = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let payload_after_len = &data[8..];
    let mut warnings = Vec::new();
    let body: Vec<u8>;
    let compressed: bool;

    if sig == "CWS" {
        let inf = decompress_zlib(payload_after_len);
        if let Some(w) = inf.warning {
            warnings.push(format!("CWS: zlib stream warning: {}", w));
        }
        if !inf.eof {
            warnings.push(format!(
                "CWS: zlib stream truncated — decoded {} bytes of {} expected (output may be incomplete)",
                inf.output.len(),
                file_length.saturating_sub(8)
            ));
        }
        body = inf.output;
        compressed = true;
    } else if sig == "ZWS" {
        if data.len() < 12 {
            return Err("ZWS: SWF too short for LZMA payload".to_string());
        }
        match lzma_decompress_zws(&data[12..]) {
            Ok(decompressed) => body = decompressed,
            Err(e) => return Err(format!("ZWS: {}", e)),
        }
        compressed = true;
    } else {
        body = payload_after_len.to_vec();
        compressed = false;
    }

    let mut br = ByteReader::new(&body, 0);
    let frame_size = read_rect(&mut br)?;
    let fps_raw = br.u16().unwrap_or(0);
    let fps = (fps_raw as f64) / 256.0;
    let frame_count = br.u16().unwrap_or(0);

    let mut header = SwfHeader {
        signature: sig.to_string(),
        version,
        file_length,
        frame_size,
        fps,
        frame_count,
        compressed,
        parse_warnings: warnings,
        symbols: HashMap::new(),
        entry_class: None,
        metadata: None,
    };

    let mut tags = Vec::new();
    loop {
        let h = match br.u16() {
            Ok(val) => val,
            Err(_) => {
                header
                    .parse_warnings
                    .push("tag stream: unexpected EOF in tag header".to_string());
                break;
            }
        };

        let tid = h >> 6;
        let mut length = (h & 0x3F) as i32;
        let mut extended = false;

        if length == 0x3F {
            length = match br.i32() {
                Ok(val) => val,
                Err(_) => {
                    header
                        .parse_warnings
                        .push("tag stream: EOF in long tag length".to_string());
                    break;
                }
            };
            extended = length < 63;
        }

        if tid == TAG_END {
            tags.push(Tag {
                tid: TAG_END,
                extended: false,
                data: Vec::new(),
            });
            break;
        }

        if length < 0 || (length as usize) > br.avail() {
            header.parse_warnings.push(format!(
                "tag stream: tag 0x{:02X} ({}) length {} exceeds remaining {} bytes — output may be incomplete",
                tid,
                tag_name(tid),
                length,
                br.avail()
            ));
            tags.push(Tag {
                tid,
                extended,
                data: br.rest().to_vec(),
            });
            break;
        }

        let tag_data = match br.bytes(length as usize) {
            Ok(bytes) => bytes.to_vec(),
            Err(_) => {
                header
                    .parse_warnings
                    .push(format!("tag stream: EOF inside tag 0x{:02X}", tid));
                break;
            }
        };

        tags.push(Tag {
            tid,
            extended,
            data: tag_data,
        });
    }

    for t in &tags {
        if (t.tid == 0x4C || t.tid == 0x38) && t.data.len() >= 2 {
            let mut sbr = ByteReader::new(&t.data, 0);
            if let Ok(count) = sbr.u16() {
                for _ in 0..count {
                    let char_id = match sbr.u16() {
                        Ok(id) => id,
                        Err(_) => break,
                    };
                    let name = match sbr.cstring() {
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    header.symbols.insert(name.clone(), char_id);
                    if t.tid == 0x4C && char_id == 0 && !name.is_empty() {
                        header.entry_class = Some(name);
                    }
                }
            }
        }
    }

    for t in &tags {
        if t.tid == 0x4D && !t.data.is_empty() {
            header.metadata = Some(String::from_utf8_lossy(&t.data).into_owned());
        }
    }

    Ok((header, tags))
}

pub fn collect_swf_info(data: &[u8]) -> Result<String, String> {
    let (header, tags) = parse_swf(data)?;
    let mut lines = Vec::new();
    lines.push(format!(
        "SWF v{} {} {:.0}x{:.0} fps={} frames={} compressed={}",
        header.version,
        header.signature,
        (header.frame_size.left as f64) / 20.0,
        (header.frame_size.top as f64) / 20.0,
        crate::haxe_out::py_repr_f64(header.fps),
        header.frame_count,
        header.compressed
    ));

    for w in &header.parse_warnings {
        lines.push(format!("WARNING: {}", w));
    }
    if let Some(ref entry) = header.entry_class {
        lines.push(format!("Entry class: {}", entry));
    }
    if !header.symbols.is_empty() {
        lines.push("Symbols:".to_string());
        for (name, tid) in &header.symbols {
            lines.push(format!("  {} -> character/tag #{}", name, tid));
        }
    }
    if let Some(ref meta) = header.metadata {
        let preview = if meta.len() > 200 {
            &meta[..200]
        } else {
            meta.as_str()
        };
        lines.push(format!("Metadata: {}", preview));
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for t in &tags {
        let nm = tag_name(t.tid);
        *counts.entry(nm).or_insert(0) += 1;
    }

    lines.push(format!("Tags: {}", tags.len()));
    let mut sorted_counts: Vec<(String, usize)> = counts.into_iter().collect();
    sorted_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (nm, n) in sorted_counts {
        lines.push(format!("  {}: {}", nm, n));
    }

    for t in &tags {
        if t.tid == TAG_FILESATTRIBUTES && !t.data.is_empty() {
            let b0 = t.data[0];
            let mut flags = Vec::new();
            if (b0 & 0x10) != 0 {
                flags.push("ActionScript3");
            }
            if (b0 & 0x08) != 0 {
                flags.push("HasMetadata");
            }
            if (b0 & 0x01) != 0 {
                flags.push("UseNetwork");
            }
            lines.push(format!(
                "FileAttributes: {}",
                if flags.is_empty() {
                    "none".to_string()
                } else {
                    flags.join(", ")
                }
            ));
        } else if t.tid == TAG_PRODUCTINFO && t.data.len() >= 12 {
            let eid = u32::from_le_bytes([t.data[0], t.data[1], t.data[2], t.data[3]]);
            let major = t.data[4];
            let minor = t.data[5];
            let build = u32::from_le_bytes([t.data[6], t.data[7], t.data[8], t.data[9]]);
            let comp = t.data[11];
            lines.push(format!(
                "ProductInfo: edition={} version={}.{} build={} compiler=Flash/{}",
                eid, major, minor, build, comp
            ));
        } else if t.tid == TAG_DEBUGID {
            let hex = t
                .data
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>();
            lines.push(format!("DebugID: {}", hex));
        } else if t.tid == TAG_SCRIPTLIMITS && t.data.len() >= 4 {
            let mr = u16::from_le_bytes([t.data[0], t.data[1]]);
            let rd = u16::from_le_bytes([t.data[2], t.data[3]]);
            lines.push(format!("ScriptLimits: maxRecursion={} timeout={}s", mr, rd));
        } else if t.tid == TAG_PROTECT {
            let hash_str = if !t.data.is_empty() {
                let n = t.data.len().min(16);
                let hex = t.data[..n]
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>();
                format!(" (password hash: {}...)", hex)
            } else {
                "".to_string()
            };
            lines.push(format!("Protect: yes{}", hash_str));
        } else if t.tid == TAG_BINARYDATA && t.data.len() >= 6 {
            let cid = u16::from_le_bytes([t.data[0], t.data[1]]);
            let size = u32::from_le_bytes([t.data[4], t.data[5], t.data[6], t.data[7]]);
            lines.push(format!("DefineBinaryData: id={} size={}", cid, size));
        } else if t.tid == TAG_DOABCDEFINE || t.tid == TAG_DOABC {
            let label = if t.tid == TAG_DOABCDEFINE {
                "DoABC(main)"
            } else {
                "DoABC"
            };
            let mut extra = String::new();
            if t.tid == TAG_DOABC && t.data.len() > 4 {
                let mut abr = ByteReader::new(&t.data, 0);
                let _ = abr.u32();
                if let Ok(nm2) = abr.cstring() {
                    extra = format!(" name={:?}", nm2);
                }
            }
            lines.push(format!("{}: {} bytes{}", label, t.data.len(), extra));
        }
    }

    Ok(lines.join("\n"))
}

pub fn iter_abc_tags(tags: &[Tag]) -> Vec<(Option<(u32, String)>, &[u8])> {
    let mut out = Vec::new();
    for t in tags {
        if t.tid == TAG_DOABCDEFINE {
            out.push((None, t.data.as_slice()));
        } else if t.tid == TAG_DOABC && t.data.len() >= 4 {
            let mut br = ByteReader::new(&t.data, 0);
            let tag_id = br.u32().unwrap_or(0);
            let frame = br.cstring().unwrap_or_default();
            let rest = br.rest();
            out.push((Some((tag_id, frame)), rest));
        }
    }
    out
}

pub fn zlib_compress_for_test(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 16);
    out.push(0x78);
    out.push(0x9C);
    let mut chunks = data.chunks(65535).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }
    while let Some(chunk) = chunks.next() {
        let bfinal = if chunks.peek().is_none() { 1u8 } else { 0u8 };
        out.push(bfinal);
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    let adler = (b << 16) | a;
    out.extend_from_slice(&adler.to_be_bytes());
    out
}
