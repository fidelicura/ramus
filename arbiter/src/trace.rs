// Copyright 2026 Ramus
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use flate2::read::GzDecoder;
use ramus::{Info, Kind, Outcome, Source, Value};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Trace {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A CBP-NG 2026 gzipped instruction trace, decoded into memory.
#[derive(Clone)]
pub struct Trace {
    records: Arc<[Info]>,
    cursor: usize,
}

impl Trace {
    pub fn open<P>(path: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        let mut file = File::open(path)?;

        let mut trailer = [0u8; 4];
        file.seek(SeekFrom::End(-4))?;
        file.read_exact(&mut trailer)?;
        file.seek(SeekFrom::Start(0))?;
        let size = u32::from_le_bytes(trailer) as usize;

        let mut reader = Reader {
            reader: BufReader::new(GzDecoder::new(file)),
        };

        let mut records = Vec::with_capacity(size);
        while let Some(info) = reader.next() {
            records.push(info);
        }

        Ok(Self {
            records: records.into(),
            cursor: 0,
        })
    }
}

impl Source for Trace {
    fn fetch(&mut self) -> Option<Info> {
        let info = self.records.get(self.cursor).copied()?;
        self.cursor += 1;
        Some(info)
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Trace
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Reader {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Streaming parser over the decompressed byte stream.
struct Reader {
    reader: BufReader<GzDecoder<File>>,
}

impl Reader {
    fn read_byte(&mut self) -> Option<u8> {
        let mut buffer = [0u8; 1];
        self.reader.read_exact(&mut buffer).ok()?;
        Some(buffer[0])
    }

    fn read_word(&mut self) -> Option<u64> {
        let mut buffer = [0u8; 8];
        self.reader.read_exact(&mut buffer).ok()?;
        Some(u64::from_le_bytes(buffer))
    }

    fn read_branch(&mut self, address: u64) -> Option<(Outcome, u64)> {
        let taken = self.read_byte()? != 0;
        let next = if taken {
            self.read_word()?
        } else {
            address + 4
        };
        Some((Outcome::from(taken), next))
    }

    fn skip_amount(&mut self, count: u64) -> Option<()> {
        let reader = &mut self.reader.by_ref().take(count);
        let writer = &mut io::sink();
        let read = io::copy(reader, writer).ok()?;
        (read == count).then_some(())
    }

    fn skip_registers(&mut self) -> Option<()> {
        // Inputs: count, then one name byte each.
        let inputs = self.read_byte()?;
        self.skip_amount(inputs as u64)?;

        // Outputs: count, then all names, then all values.
        let outputs = self.read_byte()?;
        let mut names = [0u8; u8::MAX as usize + 1];
        let names = &mut names[..outputs as usize];
        self.reader.read_exact(names).ok()?;
        for &name in names.iter() {
            // SIMD regs (32..=63) carry 16-byte values, others 8.
            let size = if (32..=63).contains(&name) { 16 } else { 8 };
            self.skip_amount(size)?;
        }
        Some(())
    }

    fn next(&mut self) -> Option<Info> {
        loop {
            let address = self.read_word()?;
            let class = self.read_byte()?;
            let kind = match class {
                1 /* LOAD */ => {
                    self.skip_amount(10)?;
                    None
                },
                2 /* STORE */ => {
                    self.skip_amount(11)?;
                    None
                },
                3 => Some(Kind::Conditional),
                4 => Some(Kind::UnconditionalDirect),
                5 => Some(Kind::UnconditionalIndirect),
                9 => Some(Kind::CallDirect),
                10 => Some(Kind::CallIndirect),
                11 => Some(Kind::Return),
                _ => None,
            };
            let branch = if kind.is_some() {
                Some(self.read_branch(address)?)
            } else {
                None
            };
            self.skip_registers()?;
            if let (Some(kind), Some((outcome, next))) = (kind, branch) {
                return Some(Info {
                    address: Value::failing(address).unwrap(),
                    outcome,
                    kind,
                    next: Value::failing(next).unwrap(),
                });
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Reader
////////////////////////////////////////////////////////////////////////////////////////////////////
