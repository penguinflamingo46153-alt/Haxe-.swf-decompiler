#[derive(Debug, Clone)]
pub struct InflateResult {
    pub output: Vec<u8>,
    pub eof: bool,
    pub warning: Option<String>,
}

struct BitStream<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u32,
    bits_in_buf: u8,
}

impl<'a> BitStream<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bits_in_buf: 0,
        }
    }

    fn ensure_bits(&mut self, n: u8) {
        while self.bits_in_buf < n && self.byte_pos < self.data.len() {
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bits_in_buf;
            self.byte_pos += 1;
            self.bits_in_buf += 8;
        }
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, ()> {
        self.ensure_bits(n);
        if self.bits_in_buf < n {
            return Err(());
        }
        let mask = (1u32 << n) - 1;
        let val = self.bit_buf & mask;
        self.bit_buf >>= n;
        self.bits_in_buf -= n;
        Ok(val)
    }

    #[allow(dead_code)]
    fn drop_bits(&mut self, n: u8) {
        self.bit_buf >>= n;
        self.bits_in_buf = self.bits_in_buf.saturating_sub(n);
    }

    #[allow(dead_code)]
    fn peek_bits(&mut self, n: u8) -> u32 {
        self.ensure_bits(n);
        let mask = (1u32 << n) - 1;
        self.bit_buf & mask
    }

    fn align_byte(&mut self) {
        let rem = self.bits_in_buf % 8;
        self.bit_buf >>= rem;
        self.bits_in_buf -= rem;
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], ()> {
        self.align_byte();
        while self.bits_in_buf >= 8 {
            self.bits_in_buf -= 8;
            self.bit_buf >>= 8;
            self.byte_pos -= 1;
        }
        if self.byte_pos + n > self.data.len() {
            return Err(());
        }
        let slice = &self.data[self.byte_pos..self.byte_pos + n];
        self.byte_pos += n;
        Ok(slice)
    }
}

#[derive(Debug, Clone)]
struct HuffmanTree {
    table: Vec<u16>,
    #[allow(dead_code)]
    max_bits: u8,
}

impl HuffmanTree {
    fn from_lengths(lengths: &[u8]) -> Result<Self, ()> {
        let max_len = (*lengths.iter().max().unwrap_or(&0)).min(15);
        if max_len == 0 {
            return Ok(Self {
                table: vec![0; 1],
                max_bits: 0,
            });
        }

        let mut bl_count = [0usize; 16];
        for &l in lengths {
            if l > 0 && (l as usize) <= 15 {
                bl_count[l as usize] += 1;
            }
        }

        let mut next_code = [0u16; 16];
        let mut code = 0u16;
        for bits in 1..=15 {
            code = (code + bl_count[bits - 1] as u16) << 1;
            next_code[bits] = code;
        }

        let mut tree = vec![0u16; 2];
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let c = next_code[len as usize];
            next_code[len as usize] += 1;

            let mut node_idx = 0usize;
            for bit_pos in (0..len).rev() {
                let bit = (c >> bit_pos) & 1;
                let child_slot = node_idx * 2 + (bit as usize);
                if child_slot >= tree.len() {
                    tree.resize(child_slot + 2, 0);
                }
                if bit_pos == 0 {
                    tree[child_slot] = 0x8000 | (sym as u16);
                } else {
                    let mut child = tree[child_slot];
                    if child == 0 || (child & 0x8000) != 0 {
                        let new_node = tree.len() / 2;
                        tree.resize(tree.len() + 2, 0);
                        tree[child_slot] = new_node as u16;
                        child = new_node as u16;
                    }
                    node_idx = child as usize;
                }
            }
        }

        Ok(Self {
            table: tree,
            max_bits: max_len,
        })
    }

    fn decode(&self, bs: &mut BitStream) -> Result<u16, ()> {
        let mut node = 0usize;
        for _ in 0..15 {
            let bit = bs.read_bits(1)? as usize;
            let child_slot = node * 2 + bit;
            if child_slot >= self.table.len() {
                return Err(());
            }
            let val = self.table[child_slot];
            if (val & 0x8000) != 0 {
                return Ok(val & 0x7FFF);
            }
            if val == 0 {
                return Err(());
            }
            node = val as usize;
        }
        Err(())
    }
}

fn build_fixed_trees() -> (HuffmanTree, HuffmanTree) {
    let mut lit_lens = [0u8; 288];
    for i in 0..=143 {
        lit_lens[i] = 8;
    }
    for i in 144..=255 {
        lit_lens[i] = 9;
    }
    for i in 256..=279 {
        lit_lens[i] = 7;
    }
    for i in 280..=287 {
        lit_lens[i] = 8;
    }
    let dist_lens = [5u8; 32];
    (
        HuffmanTree::from_lengths(&lit_lens).unwrap(),
        HuffmanTree::from_lengths(&dist_lens).unwrap(),
    )
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
    131, 163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
    13, 13,
];

const CODE_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

pub fn decompress_zlib(data: &[u8]) -> InflateResult {
    if data.len() < 2 {
        return InflateResult {
            output: Vec::new(),
            eof: false,
            warning: Some("Zlib data too short".to_string()),
        };
    }

    let cmf = data[0];
    let flg = data[1];
    let is_zlib = (cmf & 0x0F) == 8 && (((cmf as u16) * 256 + (flg as u16)) % 31 == 0);

    let deflate_data = if is_zlib {
        let mut offset = 2;
        if (flg & 0x20) != 0 {
            offset += 4;
        }
        if offset <= data.len() {
            &data[offset..]
        } else {
            &[]
        }
    } else {
        data
    };

    let mut bs = BitStream::new(deflate_data);
    let mut out = Vec::new();
    let mut is_final = false;
    let mut error = None;

    let (fixed_lit, fixed_dist) = build_fixed_trees();

    while !is_final {
        let bfinal = match bs.read_bits(1) {
            Ok(b) => b != 0,
            Err(_) => {
                error = Some("Truncated at block header".to_string());
                break;
            }
        };
        is_final = bfinal;

        let btype = match bs.read_bits(2) {
            Ok(t) => t,
            Err(_) => {
                error = Some("Truncated at block type".to_string());
                break;
            }
        };

        match btype {
            0 => {
                bs.align_byte();
                let len = match bs.read_bits(16) {
                    Ok(l) => l as usize,
                    Err(_) => {
                        error = Some("Truncated stored block len".to_string());
                        break;
                    }
                };
                let _nlen = match bs.read_bits(16) {
                    Ok(nl) => nl as usize,
                    Err(_) => {
                        error = Some("Truncated stored block nlen".to_string());
                        break;
                    }
                };
                let bytes = match bs.read_bytes(len) {
                    Ok(b) => b,
                    Err(_) => {
                        error = Some("Truncated stored block payload".to_string());
                        break;
                    }
                };
                out.extend_from_slice(bytes);
            }
            1 => {
                if let Err(e) = decode_huffman_block(&mut bs, &fixed_lit, &fixed_dist, &mut out) {
                    error = Some(e);
                    break;
                }
            }
            2 => {
                let hlit = match bs.read_bits(5) {
                    Ok(v) => (v + 257) as usize,
                    Err(_) => {
                        error = Some("Truncated dynamic hlit".to_string());
                        break;
                    }
                };
                let hdist = match bs.read_bits(5) {
                    Ok(v) => (v + 1) as usize,
                    Err(_) => {
                        error = Some("Truncated dynamic hdist".to_string());
                        break;
                    }
                };
                let hclen = match bs.read_bits(4) {
                    Ok(v) => (v + 4) as usize,
                    Err(_) => {
                        error = Some("Truncated dynamic hclen".to_string());
                        break;
                    }
                };

                let mut code_lengths = [0u8; 19];
                for i in 0..hclen {
                    code_lengths[CODE_ORDER[i]] = match bs.read_bits(3) {
                        Ok(v) => v as u8,
                        Err(_) => {
                            error = Some("Truncated code lengths".to_string());
                            break;
                        }
                    };
                }
                if error.is_some() {
                    break;
                }

                let code_tree = match HuffmanTree::from_lengths(&code_lengths) {
                    Ok(t) => t,
                    Err(_) => {
                        error = Some("Invalid code lengths tree".to_string());
                        break;
                    }
                };

                let mut lit_dist_lens = Vec::with_capacity(hlit + hdist);
                while lit_dist_lens.len() < hlit + hdist {
                    let sym = match code_tree.decode(&mut bs) {
                        Ok(s) => s,
                        Err(_) => {
                            error = Some("Truncated lit/dist lengths".to_string());
                            break;
                        }
                    };
                    if sym < 16 {
                        lit_dist_lens.push(sym as u8);
                    } else if sym == 16 {
                        let repeat = match bs.read_bits(2) {
                            Ok(v) => (v + 3) as usize,
                            Err(_) => {
                                error = Some("Truncated repeat 16".to_string());
                                break;
                            }
                        };
                        let last = *lit_dist_lens.last().unwrap_or(&0);
                        for _ in 0..repeat {
                            lit_dist_lens.push(last);
                        }
                    } else if sym == 17 {
                        let repeat = match bs.read_bits(3) {
                            Ok(v) => (v + 3) as usize,
                            Err(_) => {
                                error = Some("Truncated repeat 17".to_string());
                                break;
                            }
                        };
                        for _ in 0..repeat {
                            lit_dist_lens.push(0);
                        }
                    } else if sym == 18 {
                        let repeat = match bs.read_bits(7) {
                            Ok(v) => (v + 11) as usize,
                            Err(_) => {
                                error = Some("Truncated repeat 18".to_string());
                                break;
                            }
                        };
                        for _ in 0..repeat {
                            lit_dist_lens.push(0);
                        }
                    }
                }
                if error.is_some() {
                    break;
                }

                let dyn_lit = match HuffmanTree::from_lengths(&lit_dist_lens[..hlit]) {
                    Ok(t) => t,
                    Err(_) => {
                        error = Some("Invalid dynamic lit tree".to_string());
                        break;
                    }
                };
                let dyn_dist = match HuffmanTree::from_lengths(&lit_dist_lens[hlit..]) {
                    Ok(t) => t,
                    Err(_) => {
                        error = Some("Invalid dynamic dist tree".to_string());
                        break;
                    }
                };

                if let Err(e) = decode_huffman_block(&mut bs, &dyn_lit, &dyn_dist, &mut out) {
                    error = Some(e);
                    break;
                }
            }
            _ => {
                error = Some("Unknown block type (3)".to_string());
                break;
            }
        }
    }

    InflateResult {
        output: out,
        eof: is_final && error.is_none(),
        warning: error,
    }
}

fn decode_huffman_block(
    bs: &mut BitStream,
    lit_tree: &HuffmanTree,
    dist_tree: &HuffmanTree,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    loop {
        let sym = lit_tree.decode(bs).map_err(|_| "Truncated literal".to_string())?;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            break;
        } else if sym <= 285 {
            let len_idx = (sym - 257) as usize;
            let mut length = LENGTH_BASE[len_idx] as usize;
            let extra_bits = LENGTH_EXTRA[len_idx];
            if extra_bits > 0 {
                let extra = bs
                    .read_bits(extra_bits)
                    .map_err(|_| "Truncated length extra".to_string())?;
                length += extra as usize;
            }

            let dist_sym = dist_tree
                .decode(bs)
                .map_err(|_| "Truncated distance".to_string())? as usize;
            if dist_sym >= 30 {
                return Err(format!("Invalid distance symbol {}", dist_sym));
            }
            let mut dist = DIST_BASE[dist_sym] as usize;
            let dist_extra = DIST_EXTRA[dist_sym];
            if dist_extra > 0 {
                let extra = bs
                    .read_bits(dist_extra)
                    .map_err(|_| "Truncated distance extra".to_string())?;
                dist += extra as usize;
            }

            if dist > out.len() {
                return Err(format!(
                    "Distance {} exceeds output buffer {}",
                    dist,
                    out.len()
                ));
            }

            let start = out.len() - dist;
            for i in 0..length {
                let b = out[start + i];
                out.push(b);
            }
        } else {
            return Err(format!("Invalid literal symbol {}", sym));
        }
    }
    Ok(())
}

const NUM_BIT_MODEL_TOTAL_BITS: u32 = 11;
const BIT_MODEL_TOTAL: u32 = 1 << NUM_BIT_MODEL_TOTAL_BITS;
const PROB_INIT: u16 = (BIT_MODEL_TOTAL / 2) as u16;
const NUM_MOVE_BITS: u32 = 5;
const TOP_VALUE: u32 = 1 << 24;

const P_IS_MATCH: usize = 0;
const P_IS_REP: usize = 192;
const P_IS_REP_G0: usize = 204;
const P_IS_REP_G1: usize = 216;
const P_IS_REP_G2: usize = 228;
const P_IS_REP_0_LONG: usize = 240;
const P_POS_SLOT: usize = 432;
const P_SPEC_POS: usize = 688;
const P_ALIGN: usize = 803;
const P_LEN_CODER: usize = 819;
const P_REP_LEN_CODER: usize = 1333;
const P_LITERAL: usize = 1847;

#[allow(dead_code)]
const NUM_STATES: usize = 12;
const NUM_POS_STATES: usize = 16;
const NUM_LEN_TO_POS_STATES: usize = 4;
const END_POS_MODEL_INDEX: usize = 14;
const NUM_ALIGN_BITS: u32 = 4;
const MATCH_MIN_LEN: usize = 2;

struct RangeDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
    corrupt: bool,
}

impl<'a> RangeDecoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut code: u32 = 0;
        let take = 5.min(data.len());
        for i in 0..take {
            code = (code << 8) | data[i] as u32;
        }
        RangeDecoder {
            data,
            pos: take,
            range: 0xFFFFFFFF,
            code,
            corrupt: false,
        }
    }

    #[inline]
    fn normalize(&mut self) {
        if self.range < TOP_VALUE {
            self.range <<= 8;
            let b = if self.pos < self.data.len() {
                let b = self.data[self.pos];
                self.pos += 1;
                b
            } else {
                self.corrupt = true;
                0
            };
            self.code = (self.code << 8) | b as u32;
        }
    }

    #[inline]
    fn decode_bit(&mut self, prob: &mut u16) -> u32 {
        let bound = (self.range >> NUM_BIT_MODEL_TOTAL_BITS) * (*prob as u32);
        let bit;
        if self.code < bound {
            self.range = bound;
            *prob += ((BIT_MODEL_TOTAL as u16) - *prob) >> NUM_MOVE_BITS;
            bit = 0;
        } else {
            self.range -= bound;
            self.code -= bound;
            *prob -= *prob >> NUM_MOVE_BITS;
            bit = 1;
        }
        self.normalize();
        bit
    }

    fn decode_direct_bits(&mut self, num_bits: u32) -> u32 {
        let mut result: u32 = 0;
        for _ in 0..num_bits {
            self.range >>= 1;
            self.code = self.code.wrapping_sub(self.range);
            let t = 0u32.wrapping_sub(self.code >> 31);
            self.code = self.code.wrapping_add(self.range & t);
            result = (result << 1).wrapping_add(t.wrapping_add(1));
            self.normalize();
        }
        result
    }

    fn bittree_reverse(&mut self, probs: &mut [u16], num_bits: u32) -> u32 {
        let mut m: usize = 1;
        let mut symbol: u32 = 0;
        for i in 0..num_bits {
            let bit = self.decode_bit(&mut probs[m]);
            m = (m << 1) + bit as usize;
            symbol |= bit << i;
        }
        symbol
    }

    fn bittree(&mut self, probs: &mut [u16], num_bits: u32) -> u32 {
        let mut m: usize = 1;
        for _ in 0..num_bits {
            let bit = self.decode_bit(&mut probs[m]);
            m = (m << 1) + bit as usize;
        }
        (m as u32) - (1u32 << num_bits)
    }
}

struct LenCoder {
    choice: usize,
    low: usize,
    choice2: usize,
    mid: usize,
    high: usize,
}

fn decode_len(
    rc: &mut RangeDecoder,
    probs: &mut [u16],
    c: &LenCoder,
    pos_state: usize,
) -> usize {
    if rc.decode_bit(&mut probs[c.choice]) == 0 {
        let base = c.low + pos_state * 8;
        rc.bittree(&mut probs[base..base + 8], 3) as usize
    } else if rc.decode_bit(&mut probs[c.choice2]) == 0 {
        let base = c.mid + pos_state * 8;
        8 + rc.bittree(&mut probs[base..base + 8], 3) as usize
    } else {
        16 + rc.bittree(&mut probs[c.high..c.high + 256], 8) as usize
    }
}

fn lzma_decode_stream(stream: &[u8], props_byte: u8) -> Result<Vec<u8>, String> {
    if props_byte >= 225 {
        return Err(format!("Invalid LZMA properties byte {}", props_byte));
    }
    let lc = (props_byte % 9) as u32;
    let lp = ((props_byte / 9) % 5) as u32;
    let pb = (props_byte / 45) as u32;

    let lit_probs_len = 0x300usize << (lp + lc);
    let mut probs = vec![PROB_INIT; P_LITERAL + lit_probs_len];

    let len_coder = LenCoder {
        choice: P_LEN_CODER,
        low: P_LEN_CODER + 1,
        choice2: P_LEN_CODER + 1 + 128,
        mid: P_LEN_CODER + 1 + 128 + 1,
        high: P_LEN_CODER + 1 + 128 + 1 + 128,
    };
    let rep_len_coder = LenCoder {
        choice: P_REP_LEN_CODER,
        low: P_REP_LEN_CODER + 1,
        choice2: P_REP_LEN_CODER + 1 + 128,
        mid: P_REP_LEN_CODER + 1 + 128 + 1,
        high: P_REP_LEN_CODER + 1 + 128 + 1 + 128,
    };
    debug_assert_eq!(rep_len_coder.high + 256, P_LITERAL);

    let mut rc = RangeDecoder::new(stream);
    let mut out: Vec<u8> = Vec::new();
    let mut state: usize = 0;
    let mut rep0: u32 = 0;
    let mut rep1: u32 = 0;
    let mut rep2: u32 = 0;
    let mut rep3: u32 = 0;
    let pos_state_mask: u32 = (1u32 << pb) - 1;

    loop {
        if rc.corrupt {
            break;
        }
        let pos_state = (out.len() as u32 & pos_state_mask) as usize;

        if rc.decode_bit(&mut probs[P_IS_MATCH + state * NUM_POS_STATES + pos_state]) == 0 {
            let prev_byte: u32 = if out.is_empty() { 0 } else { out[out.len() - 1] as u32 };
            let lit_state = (((out.len() as u32) & ((1u32 << lp) - 1)) << lc) as usize
                + (prev_byte >> (8 - lc)) as usize;
            let lp_base = P_LITERAL + 0x300 * lit_state;
            let mut symbol: u32 = 1;
            if state >= 7 {
                let dist = rep0 as usize;
                if dist >= out.len() {
                    return Err("LZMA: literal match distance beyond output".into());
                }
                let mut match_byte = out[out.len() - dist - 1] as u32;
                while symbol < 0x100 {
                    let match_bit = (match_byte >> 7) & 1;
                    match_byte <<= 1;
                    let idx = lp_base + (((1 + match_bit) as usize) << 8) + symbol as usize;
                    let bit = rc.decode_bit(&mut probs[idx]);
                    symbol = (symbol << 1) | bit;
                    if match_bit != bit {
                        break;
                    }
                }
            }
            while symbol < 0x100 {
                let bit = rc.decode_bit(&mut probs[lp_base + symbol as usize]);
                symbol = (symbol << 1) | bit;
            }
            out.push((symbol & 0xFF) as u8);
            state = if state < 4 {
                0
            } else if state < 10 {
                state - 3
            } else {
                state - 6
            };
        } else {
            if rc.decode_bit(&mut probs[P_IS_REP + state]) != 0 {
                if rc.decode_bit(&mut probs[P_IS_REP_G0 + state]) == 0 {
                    if rc.decode_bit(&mut probs[P_IS_REP_0_LONG + state * NUM_POS_STATES + pos_state]) == 0 {
                        state = if state < 7 { 9 } else { 11 };
                        let dist = rep0 as usize;
                        if dist >= out.len() {
                            return Err("LZMA: short rep beyond output".into());
                        }
                        let b = out[out.len() - dist - 1];
                        out.push(b);
                        continue;
                    }
                } else {
                    let dist;
                    if rc.decode_bit(&mut probs[P_IS_REP_G1 + state]) == 0 {
                        dist = rep1;
                    } else {
                        if rc.decode_bit(&mut probs[P_IS_REP_G2 + state]) == 0 {
                            dist = rep2;
                        } else {
                            dist = rep3;
                            rep3 = rep2;
                        }
                        rep2 = rep1;
                    }
                    rep1 = rep0;
                    rep0 = dist;
                }
                let length = decode_len(&mut rc, &mut probs, &rep_len_coder, pos_state);
                state = if state < 7 { 8 } else { 11 };
                let dist = rep0 as usize;
                if dist >= out.len() {
                    return Err("LZMA: rep distance beyond output".into());
                }
                for _ in 0..(length + MATCH_MIN_LEN) {
                    let b = out[out.len() - dist - 1];
                    out.push(b);
                }
            } else {
                rep3 = rep2;
                rep2 = rep1;
                rep1 = rep0;
                state = if state < 7 { 7 } else { 10 };
                let length = decode_len(&mut rc, &mut probs, &len_coder, pos_state);
                let len_to_pos_state = length.min(NUM_LEN_TO_POS_STATES - 1);
                let pos_slot = rc.bittree(
                    &mut probs[P_POS_SLOT + len_to_pos_state * 64
                        ..P_POS_SLOT + (len_to_pos_state + 1) * 64],
                    6,
                );
                let mut dist: u32;
                if pos_slot < 4 {
                    dist = pos_slot;
                } else {
                    let num_direct_bits = (pos_slot >> 1) - 1;
                    dist = (2 | (pos_slot & 1)) << num_direct_bits;
                    if (pos_slot as usize) < END_POS_MODEL_INDEX {
                        let base = P_SPEC_POS + (dist as usize) - (pos_slot as usize);
                        dist += rc.bittree_reverse(
                            &mut probs[base..base + (1usize << num_direct_bits)],
                            num_direct_bits,
                        );
                    } else {
                        dist += rc
                            .decode_direct_bits(num_direct_bits - NUM_ALIGN_BITS)
                            << NUM_ALIGN_BITS;
                        dist += rc.bittree_reverse(
                            &mut probs[P_ALIGN..P_ALIGN + 16],
                            NUM_ALIGN_BITS,
                        );
                    }
                }
                if dist == 0xFFFFFFFF {
                    break;
                }
                if (dist as usize) > out.len() {
                    return Err(format!(
                        "LZMA: distance {} exceeds output {}",
                        dist,
                        out.len()
                    ));
                }
                rep0 = dist;
                let dist = rep0 as usize;
                for _ in 0..(length + MATCH_MIN_LEN) {
                    let b = out[out.len() - dist - 1];
                    out.push(b);
                }
            }
        }
    }

    Ok(out)
}

pub fn lzma_decompress_zws(props_and_stream: &[u8]) -> Result<Vec<u8>, String> {
    if props_and_stream.len() < 14 {
        return Err("ZWS: LZMA payload too short".to_string());
    }
    let props_byte = props_and_stream[0];
    lzma_decode_stream(&props_and_stream[13..], props_byte)
}
