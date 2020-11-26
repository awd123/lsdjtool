use std::fmt;
use std::convert::TryInto;

use crate::lsdj;
use crate::lsdj::BLOCK_SIZE;
use lsdj::LsdjSram;

const RLE_BYTE     : u8 = 0xc0; // $c0 in a compressed block indicates the beginning of an RLE sequence
const SPECIAL_BYTE : u8 = 0xe0; // indicates that the following byte has special meaning
const DEF_INST_BYTE: u8 = 0xf1; // $f1 after $e0 indicates default instrument
const DEF_WAVE_BYTE: u8 = 0xf0; // $f0 after $e0 indicates default wave
const EOF_BYTE     : u8 = 0xff; // $ff after $f0 indicates end of compressed SRAM

const DEF_INST_VALUES: [u8; DEF_INST_SIZE] = [0xa8, 0x00, 0x00, 0xff, 0x00, 0x00, 0x03, 0x00,
                                              0x00, 0xd0, 0x00, 0x00, 0x00, 0xf3, 0x00, 0x00];
const DEF_WAVE_VALUES: [u8; DEF_WAVE_SIZE] = [0x8e, 0xcd, 0xcc, 0xbb, 0xaa, 0xa9, 0x99, 0x88,
                                              0x87, 0x76, 0x66, 0x55, 0x54, 0x43, 0x32, 0x31];
const DEF_INST_SIZE: usize = 0x10;
const DEF_WAVE_SIZE: usize = 0x10;

/// Returns true if the slice if `data` contains the bytes representing the
/// LittleSoundDj default instrument.
fn is_def_inst(data: &[u8]) -> bool {
    let data_array: [u8; DEF_INST_SIZE] = match data.try_into() {
        Ok(arr) => arr,
        Err(_)  => return false // if slice is the wrong size
    };

    for i in 0..DEF_INST_SIZE {
        if data_array[i] != DEF_INST_VALUES[i] {
            return false;
        }
    }
    true
}

/// Returns true if the slice if `data` contains the bytes representing the
/// LittleSoundDj default wave.
fn is_def_wave(data: &[u8]) -> bool {
    let data_array: [u8; DEF_WAVE_SIZE] = match data.try_into() {
        Ok(arr) => arr,
        Err(_)  => return false
    };

    for i in 0..DEF_WAVE_SIZE {
        if data_array[i] != DEF_WAVE_VALUES[i] {
            return false;
        }
    }
    true
}

/// Represents a block of compressed LSDj song data.
#[derive(Clone, Copy)]
pub struct LsdjBlock {
    pub position: usize,
    pub data: [u8; BLOCK_SIZE],
}

impl LsdjBlock {
    /// Returns an `LsdjBlock` with all fields initialized to zero.
    pub fn empty() -> LsdjBlock {
        LsdjBlock { position: 0, data: [0; BLOCK_SIZE] }
    }

    /// Decompresses this block into a section of SRAM.
    pub fn decompress(&self, dest: &mut LsdjSram) -> Result<u8, &'static str> {
        let base = dest.position;
        let mut offset = 0;
        let mut block_index = 0;

        while block_index < lsdj::BLOCK_SIZE {
            match self.data[block_index] {
                RLE_BYTE => {
                    if self.data[block_index + 1] == RLE_BYTE {
                        dest.data[base + offset] = RLE_BYTE;
                        offset += 1;
                        block_index += 2;
                    } else {
                        block_index += 1;
                        let byte_value = self.data[block_index];
                        block_index += 1;
                        let byte_repeat = self.data[block_index];
                        for _j in 0..byte_repeat {
                            dest.data[base + offset] = byte_value;
                            offset += 1;
                        }
                        block_index += 1;
                    }
                },
                SPECIAL_BYTE => {
                    block_index += 1;
                    match self.data[block_index] {
                        SPECIAL_BYTE => {
                            dest.data[base + offset] = SPECIAL_BYTE;
                            offset += 1;
                        },
                        DEF_INST_BYTE =>
                            for j in 0..DEF_INST_SIZE {
                                dest.data[base + offset] = DEF_INST_VALUES[j];
                                offset += 1;
                            },
                        DEF_WAVE_BYTE =>
                            for j in 0..DEF_WAVE_SIZE {
                                dest.data[base + offset] = DEF_WAVE_VALUES[j];
                                offset += 1;
                            },
                        EOF_BYTE => {
                            dest.position += offset;
                            return Ok(0);
                        },
                        switch_block => {
                            dest.position += offset;
                            return Ok(switch_block);
                        },
                    }
                    block_index += 1;
                },
                byte => {
                    dest.data[base + offset] = byte;
                    offset += 1;
                    block_index += 1;
                },
            }
        }
        dest.position += offset;
        Err(lsdj::ERR_BAD_FMT)
    }

    /// Changes the "skip to block `n`" instruction ($e0, n) at the end of the
    /// block to point to the specified block.
    pub fn skip_to_block(&mut self, block: usize) -> Result<(), &'static str> {
        let mut bytes_iter = self.data.iter_mut();
        while let Some(byte) = bytes_iter.next() {
            if *byte == SPECIAL_BYTE {
                match bytes_iter.next() {
                    Some(n) if 1 <= *n && *n <= lsdj::BLOCK_COUNT as u8 || *n == b'x' => {
                        *n = block as u8; // skip to block
                        return Ok(());
                    },
                    Some(&mut DEF_INST_BYTE) | Some(&mut DEF_WAVE_BYTE) => (),
                    Some(&mut EOF_BYTE) => return Err(lsdj::ERR_NO_SKIP), // block doesn't contain a skip instruction
                    Some(_) | None => return Err(lsdj::ERR_BAD_FMT), // block contains a $c0 with no following byte
                }
            }
        }
        Err(lsdj::ERR_NO_SKIP)
    }
}

pub trait LsdjBlockExt<T> {
    /// Decompresses all blocks stored in a `Vec<LsdjBlock>`, storing the
    /// decompressed SRAM data in `dest`.
    fn decompress_to(&self, dest: &mut LsdjSram, start_index: usize) -> Result<u8, &'static str>;

    /// Returns all bytes in all blocks as a `Vec<u8>`.
    fn bytes(&self) -> Vec<u8>;
}

impl LsdjBlockExt<LsdjBlock> for Vec<LsdjBlock> {
    fn decompress_to(&self, mut dest: &mut LsdjSram, start_index: usize) -> Result<u8, &'static str> {
        let mut blocks_decompressed = 0;
        let mut current_index = start_index;

        while current_index < self.len() {
            let next_block = self[current_index].decompress(&mut dest)?;
            blocks_decompressed += 1;
            /*
            match next_block {
                Some(n) if n > 0 => current_index = (n - 1) as usize,
                Some(0) => break,
                None => return Err("error in decompression"),
                _ => (),
            }
            */
            match next_block {
                0 => break, // return value of 0 indicates end of compressed SRAM
                n => current_index = (n - 1) as usize // move to index of next block (subtracting 1 because blocks are 1-indexed)
            }
        }
        Ok(blocks_decompressed)
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for block in self.iter() {
            for byte in block.data.iter() {
                out.push(*byte);
            }
        }
        out
    }
}

impl LsdjSram {
    /// Compresses this SRAM data into block `dest`, stopping when the
    /// destination block runs out of space or the SRAM hits its end.
    fn compress(&mut self, dest: &mut LsdjBlock, block_num: u8) -> Result<u8, &'static str> {
        let base = self.position;
        let mut offset = 0;
        let mut block_index = 0;

        while base + offset < lsdj::SRAM_SIZE {
            let mut repeat = 1;
            match self.data[base + offset] {
                RLE_BYTE => {
                    dest.data[block_index] = RLE_BYTE;
                    block_index += 1;
                    dest.data[block_index] = RLE_BYTE;
                    block_index += 1;
                    offset += 1;
                },
                SPECIAL_BYTE => {
                    dest.data[block_index] = SPECIAL_BYTE;
                    block_index += 1;
                    dest.data[block_index] = SPECIAL_BYTE;
                    block_index += 1;
                    offset += 1;
                },
                _ => {
                    if block_index + 4 > lsdj::BLOCK_SIZE {
                        dest.data[block_index] = SPECIAL_BYTE;
                        block_index += 1;
                        dest.data[block_index] = block_num + 1;
                        self.position += offset;
                        return Ok(block_num + 1);
                    } else if base + offset + DEF_INST_SIZE <= lsdj::SRAM_SIZE &&
                              is_def_inst(&self.data[(base + offset)..(base + offset + DEF_INST_SIZE)]) {
                        dest.data[block_index] = SPECIAL_BYTE;
                        block_index += 1;
                        dest.data[block_index] = DEF_INST_BYTE;
                        block_index += 1;
                        offset += DEF_INST_SIZE;
                    } else if base + offset + DEF_WAVE_SIZE <= lsdj::SRAM_SIZE &&
                              is_def_wave(&self.data[(base + offset)..(base + offset + DEF_WAVE_SIZE)]) {
                        dest.data[block_index] = SPECIAL_BYTE;
                        block_index += 1;
                        dest.data[block_index] = DEF_WAVE_BYTE;
                        block_index += 1;
                        offset += DEF_INST_SIZE;
                    } else {
                        let mut lookahead = 1;
                        while base + offset + lookahead < lsdj::SRAM_SIZE && repeat < 0xff {
                            let c = self.data[base + offset];
                            let next = self.data[base + offset + lookahead];
                            if c == next {
                                repeat += 1;
                            } else {
                                break;
                            }
                            lookahead += 1;
                        }
                        if repeat <= 3 {
                            for _i in 0..repeat {
                                dest.data[block_index] = self.data[base + offset];
                                block_index += 1;
                                offset += 1;
                            }
                        } else {
                            dest.data[block_index] = RLE_BYTE;
                            block_index += 1;
                            dest.data[block_index] = self.data[base + offset];
                            block_index += 1;
                            dest.data[block_index] = repeat;
                            block_index += 1;
                            offset += repeat as usize;
                        }
                    }
                }
            }
        }
        dest.data[block_index] = SPECIAL_BYTE;
        block_index += 1;
        dest.data[block_index] = EOF_BYTE;
        self.position += offset;
        Ok(0)
    }

    /// Wrapper function for `compress()` that compresses an entire SRAM at
    /// once and stores the compressed bytes into a `Vec<LsdjBlock>`.
    pub fn compress_into(&mut self, blocks: &mut Vec<LsdjBlock>, first_block: usize) -> Result<u8, &'static str> {
        let mut current_block = first_block;
        let mut blocks_written = 0;
        loop {
            blocks.push(LsdjBlock::empty());
            let next_block = self.compress(&mut blocks[current_block - 1], current_block as u8)?;
            blocks_written += 1;
            /*
            match next_block {
                Some(n) if n > 0 => current_block = n as usize,
                Some(0) => break,
                None => return Err("error in compression"),
                _ => (),
            }
            */
            match next_block {
                0 => break,
                n => current_block = n as usize
            }
        }
        Ok(blocks_written)
    }
}

impl fmt::Debug for LsdjBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "     | ")?;
        for i in 0..0x10 {
            write!(f, "{:X} | ", i)?;
        }
        write!(f, "\n")?;
        for disp in 0..(BLOCK_SIZE / 0x10) {
            write!(f, "{:03X}  | ", disp * 0x10)?;
            for offset in 0..0x10 {
                write!(f, "{:02X}| ", self.data[disp * 0x10 + offset])?;
            }
            write!(f, "\n")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::fs::File;

    use super::*;

    #[test]
    fn test_is_def_inst() {
        let def_inst_slice = &DEF_INST_VALUES;
        let short_def_inst = &DEF_INST_VALUES[0..0xf];
        assert!(is_def_inst(def_inst_slice));
        assert!(!is_def_inst(short_def_inst));
        assert!(!is_def_inst(&[0; DEF_INST_SIZE]));
        assert!(!is_def_inst(&[0]));
        assert!(!is_def_inst(&DEF_WAVE_VALUES));
    }

    #[test]
    fn test_is_def_wave() {
        let def_wave_slice = &DEF_WAVE_VALUES;
        let short_def_wave = &DEF_WAVE_VALUES[0..0xf];
        assert!(is_def_wave(def_wave_slice));
        assert!(!is_def_wave(short_def_wave));
        assert!(!is_def_wave(&[0; DEF_WAVE_SIZE]));
        assert!(!is_def_wave(&[0]));
        assert!(!is_def_wave(&DEF_INST_VALUES));
    }

    #[test]
    fn test_rle_decompression() {
        let mut block = LsdjBlock::empty();
        block.data[0] = 0xc0;
        block.data[1] = 0x41;
        block.data[2] = 0x10;
        let mut sram = LsdjSram::empty();
        block.decompress(&mut sram);
        // SRAM should be 0x41, repeated 16 times
        assert_eq!(&sram.data[0..0x10], &[0x41; 0x10]);
    }

    #[test]
    fn test_rle_compression() {
        let mut sram  = LsdjSram::empty();
        sram.data[0]  = 0x41;
        sram.data[1]  = 0x41;
        sram.data[2]  = 0x41;
        sram.data[3]  = 0x41;
        sram.data[4]  = 0x41;
        sram.data[5]  = 0x41;
        sram.data[6]  = 0x41;
        sram.data[7]  = 0x41;
        sram.data[8]  = 0x41;
        sram.data[9]  = 0x41;
        sram.data[10] = 0x41;
        sram.data[11] = 0x41;
        sram.data[12] = 0x41;
        sram.data[13] = 0x41;
        sram.data[14] = 0x41;
        sram.data[15] = 0x41;
        sram.data[16] = 0x41;
        sram.data[17] = 0x41;
        let mut block = LsdjBlock::empty();
        sram.compress(&mut block, 1);
        assert_eq!(&block.data[0..3], &[0xc0, 0x41, 18]);
    }


    #[test]
    fn check_sram_compression() -> std::io::Result<()> {
        let savepath = PathBuf::from("saves/test.sav");
        let mut savefile = File::open(savepath)?;
        let mut blocks: Vec<LsdjBlock> = Vec::new();
        let mut sram = LsdjSram::from(&mut savefile)?;
        sram.compress_into(&mut blocks, 1);
        let mut decompressed_sram = LsdjSram::empty();
        blocks.decompress_to(&mut decompressed_sram, 0);
        assert_eq!(sram, decompressed_sram);
        Ok(())
    }

    #[test]
    fn test_skip_to_block() {
        let mut empty_block = LsdjBlock::empty();
        assert_eq!(empty_block.skip_to_block(0xb), Err(lsdj::ERR_NO_SKIP));
        let mut real_block = LsdjBlock::empty();
        real_block.data[5] = SPECIAL_BYTE;
        real_block.data[6] = 4;
        assert_eq!(real_block.skip_to_block(0xb), Ok(()));
        assert_eq!(&real_block.data[5..7], &[SPECIAL_BYTE, 0xb]);
    }
}
