#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub bitpos: u8,
    pub cur: u8,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self {
            data,
            pos,
            bitpos: 0,
            cur: 0,
        }
    }

    pub fn read_bits(&mut self, mut n: u32) -> u32 {
        let mut result: u32 = 0;
        while n > 0 {
            if self.bitpos == 0 {
                if self.pos < self.data.len() {
                    self.cur = self.data[self.pos];
                    self.pos += 1;
                } else {
                    self.cur = 0;
                }
                self.bitpos = 8;
            }
            let take = (n as u8).min(self.bitpos);
            self.bitpos -= take;
            result = (result << take) | (((self.cur >> self.bitpos) as u32) & ((1 << take) - 1));
            n -= take as u32;
        }
        result
    }

    pub fn read_sbits(&mut self, n: u32) -> i32 {
        let v = self.read_bits(n) as i32;
        if n > 0 && (v & (1 << (n - 1))) != 0 {
            v - (1 << n)
        } else {
            v
        }
    }

    pub fn align(&mut self) {
        self.bitpos = 0;
    }

    pub fn remaining_bytes(&mut self) -> &'a [u8] {
        self.align();
        if self.pos <= self.data.len() {
            &self.data[self.pos..]
        } else {
            &[]
        }
    }

    pub fn tell(&self) -> (usize, u8) {
        (self.pos, self.bitpos)
    }
}

#[derive(Debug, Clone)]
pub struct ByteReader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    pub fn avail(&self) -> usize {
        if self.pos < self.data.len() {
            self.data.len() - self.pos
        } else {
            0
        }
    }

    pub fn tell(&self) -> usize {
        self.pos
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn u8(&mut self) -> Result<u8, String> {
        if self.pos >= self.data.len() {
            return Err(format!("EOF reading u8 at offset {}", self.pos));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    pub fn s8(&mut self) -> Result<i8, String> {
        Ok(self.u8()? as i8)
    }

    pub fn u16(&mut self) -> Result<u16, String> {
        if self.pos + 2 > self.data.len() {
            return Err(format!("EOF reading u16 at offset {}", self.pos));
        }
        let bytes = [self.data[self.pos], self.data[self.pos + 1]];
        self.pos += 2;
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn s16(&mut self) -> Result<i16, String> {
        if self.pos + 2 > self.data.len() {
            return Err(format!("EOF reading s16 at offset {}", self.pos));
        }
        let bytes = [self.data[self.pos], self.data[self.pos + 1]];
        self.pos += 2;
        Ok(i16::from_le_bytes(bytes))
    }

    pub fn u24(&mut self) -> Result<u32, String> {
        if self.pos + 3 > self.data.len() {
            return Err(format!("EOF reading u24 at offset {}", self.pos));
        }
        let b = &self.data[self.pos..self.pos + 3];
        self.pos += 3;
        Ok((b[0] as u32) | ((b[1] as u32) << 8) | ((b[2] as u32) << 16))
    }

    pub fn s24(&mut self) -> Result<i32, String> {
        let v = self.u24()?;
        if (v & 0x800000) != 0 {
            Ok((v as i32) - 0x1000000)
        } else {
            Ok(v as i32)
        }
    }

    pub fn u32(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.data.len() {
            return Err(format!("EOF reading u32 at offset {}", self.pos));
        }
        let bytes = [
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ];
        self.pos += 4;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn i32(&mut self) -> Result<i32, String> {
        if self.pos + 4 > self.data.len() {
            return Err(format!("EOF reading i32 at offset {}", self.pos));
        }
        let bytes = [
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ];
        self.pos += 4;
        Ok(i32::from_le_bytes(bytes))
    }

    pub fn f64(&mut self) -> Result<f64, String> {
        if self.pos + 8 > self.data.len() {
            return Err(format!("EOF reading f64 at offset {}", self.pos));
        }
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.data[self.pos..self.pos + 8]);
        self.pos += 8;
        Ok(f64::from_le_bytes(bytes))
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.data.len() {
            return Err(format!(
                "Expected {} bytes, got {} remaining at offset {}",
                n,
                self.avail(),
                self.pos
            ));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let slice = if self.pos < self.data.len() {
            &self.data[self.pos..]
        } else {
            &[]
        };
        self.pos = self.data.len();
        slice
    }

    pub fn cstring(&mut self) -> Result<String, String> {
        let start = self.pos;
        let mut end = start;
        while end < self.data.len() && self.data[end] != 0 {
            end += 1;
        }
        if end >= self.data.len() {
            self.pos = self.data.len();
            Ok(String::from_utf8_lossy(&self.data[start..end]).into_owned())
        } else {
            let slice = &self.data[start..end];
            self.pos = end + 1;
            Ok(String::from_utf8_lossy(slice).into_owned())
        }
    }

    pub fn string(&mut self, n: usize) -> Result<String, String> {
        let b = self.bytes(n)?;
        Ok(String::from_utf8_lossy(b).into_owned())
    }
}
