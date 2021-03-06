use std::io;
use std::io::{Seek, SeekFrom::Start};
use std::io::Read;
use std::fs::File;
use std::fmt;

use compression::LsdjBlock;
use metadata::*;
use metadata::LsdjTitle;

const BLOCK_SIZE: usize = 0x200;
const BLOCK_COUNT   : usize = 0xbe;
const BANK_SIZE : usize = 0x2000;
const BANK_COUNT: usize = 4;
const SRAM_SIZE : usize = BANK_SIZE * BANK_COUNT;
const BLOCK_ADDRESS : u64   = 0x8200;
const SAVE_SIZE     : usize = 0x20000;

mod compression;
mod metadata;

pub use compression::LsdjBlockExt;
pub use metadata::lsdjtitle_from;

mod err {
    pub const SONGS_FULL   : &str = "song slots full!";
    pub const BAD_FMT      : &str = "blocks are incorrectly formatted!";
    pub const NO_BLOCKS    : &str = "not enough free blocks left!";
    pub const BLOCK_TAKEN  : &str = "block is already taken!";
    pub const NO_SKIP      : &str = "block contains no skip instruction!";
    pub const WTF          : &str = "something has gone terribly wrong";
    pub const BAD_TITLE_FMT: &str = "title must be at most 8 characters, A-Z0-9x.";
}

/// Contains the contents of LSDj's save RAM ($8000 bytes long).
pub struct LsdjSram {
    pub position: usize,
    pub data: [u8; SRAM_SIZE],
}

/// Reads blocks of compressed song data into a `Vec<u8>`, returns either an
/// `Err` or the number of blocks read.
pub fn read_blocks_from_file(mut blockfile: &mut File, mut bytes: &mut Vec<u8>) -> io::Result<usize> {
    let read_size = BLOCK_SIZE; // read a block ($200 bytes) at a time
    let mut blocks_read = 0;
    loop {
        let nread = Read::by_ref(&mut blockfile).take(read_size as u64).read_to_end(&mut bytes)?;
        blocks_read += 1;
        if nread == 0 || nread < read_size { break; }
    }
    Ok(blocks_read)
}

impl LsdjSram {
    /// Returns an `LsdjSram` with all fields initalized to zero.
    pub fn empty() -> LsdjSram {
        LsdjSram { position: 0, data: [0; SRAM_SIZE] }
    }

    /// Loads SRAM from the LSDj save file pointed to by `savefile`.
    fn load(&mut self, savefile: &mut File) -> io::Result<()> {
        savefile.seek(Start(0))?;
        let mut handle = Read::by_ref(savefile).take(SRAM_SIZE as u64);
        handle.read(&mut self.data)?;
        Ok(())
    }

    /// Creates a new `LsdjSram` by reading its data from `savefile`.
    pub fn from(mut savefile: &mut File) -> io::Result<LsdjSram> {
        let mut sram = LsdjSram::empty();
        sram.load(&mut savefile)?;
        Ok(sram)
    }
}

/// Contains a representation of all parts of an LSDj save file (the SRAM, the metadata, and the
/// blocks.)
pub struct LsdjSave {
    sram: LsdjSram,
    pub metadata: LsdjMetadata,
    blocks: LsdjBlockTable
}

impl LsdjSave {
    /// Creates an empty `LsdjSave` (all fields initialized with `::empty()`.)
    #[allow(dead_code)]
    pub fn empty() -> LsdjSave {
        LsdjSave {
            sram: LsdjSram::empty(),
            metadata: LsdjMetadata::empty(),
            blocks: LsdjBlockTable([LsdjBlock::empty(); BLOCK_COUNT])
        }
    }

    /// Creates a new `LsdjSave`, reading all data from `savefile`.
    pub fn from(mut savefile: &mut File) -> io::Result<LsdjSave> {
        let sram     = LsdjSram::from(&mut savefile)?;
        let metadata = LsdjMetadata::from(&mut savefile)?;
        let blocks   = LsdjBlockTable::from(&mut savefile)?;
        Ok(LsdjSave { sram: sram, metadata: metadata, blocks: blocks })
    }

    /// Compresses the SRAM contained in this instance, storing the compressed
    /// blocks in a `Vec<LsdjBlock>`. `first_block` is the index from which
    /// skip instructions (`$e0 xx`) are calculated.
    pub fn compress_sram_into(&mut self, mut blocks: &mut Vec<LsdjBlock>, first_block: usize) -> Result<u8, &'static str> {
        let block = self.sram.compress_into(&mut blocks, first_block)?;
        Ok(block)
    }

    /// Extracts the song at the given index to a `Vec<u8>`.
    ///
    /// # Notes
    ///
    /// Note that this function does not check whether there is actually a song
    /// at index `song`, and thus may return a `Vec` of zeroes if given a
    /// nonexistent song.
    pub fn export_song(&self, song: u8) -> Vec<u8> {
        let num_blocks = self.metadata.size_of(song);
        let mut bytes  = Vec::with_capacity(num_blocks * BLOCK_SIZE); // raw bytes from blocks
        let mut blocks = Vec::with_capacity(num_blocks); // contains LsdjBlocks
        for i in 0..blocks.capacity() {
            let next_block = match self.metadata.next_block_for(song, i) {
                Some(b) => b - 1, // blocks are one-indexed
                None => break
            };
            blocks.push(self.blocks.0[next_block]);
        }
        for block in blocks {
            for byte in block.data.iter() {
                bytes.push(*byte); // copy byte from blocks to bytes
            }
        }
        bytes
    }

    /// Adds a new song to the save file, reading from a slice of `u8`s and
    /// giving it the title specified by `title`. This function adds the song
    /// at the next available index (next unused song), or returns an `Err` if
    /// all songs are taken or there are not enough bytes left in the save file
    /// to store the blocks of song data.
    pub fn import_song(&mut self, bytes: &[u8], title: LsdjTitle) -> Result<u8, &'static str> {
        let song = match self.metadata.next_available_song() {
            Some(s) => s,
            None => return Err(err::SONGS_FULL)
        };
        if bytes.len() % BLOCK_SIZE != 0 {
            return Err(err::BAD_FMT); // make sure correct number of bytes are passed in
        }
        let num_blocks  = bytes.len() / BLOCK_SIZE;
        let free_blocks = BLOCK_COUNT - self.metadata.blocks_used();
        if num_blocks > free_blocks {
            return Err(err::NO_BLOCKS);
        }
        let mut blocks_vec = Vec::with_capacity(num_blocks);
        for i in 0..blocks_vec.capacity() {
            let start = i * BLOCK_SIZE; // index to begin copying bytes from
            let end   = start + BLOCK_SIZE; // where to stop fetching blocks
            let mut bytes_array = [0; BLOCK_SIZE];
            for (index, byte) in bytes[start..end].iter().enumerate() {
                bytes_array[index] = *byte;
            } // copy bytes from slice into array to allow using in an LsdjBlock
            blocks_vec.push(LsdjBlock {
                position: 0,
                data: bytes_array
            });
        }
        let mut block_positions = Vec::with_capacity(num_blocks);
        for _block in blocks_vec.iter() {
            if let Some(next_block) = self.metadata.next_empty_block() {
                self.metadata.reserve(next_block, song)?;
                block_positions.push(next_block); // keep track of reserved blocks so that we know where to insert song data
            }
        }
        let mut positions_iter = block_positions.iter().peekable();
        let mut blocks_iter    = blocks_vec.iter_mut().enumerate();
        while let (Some(pos), Some((num_copied, block))) =
                  (positions_iter.next(), blocks_iter.next()) {
            if num_copied < num_blocks - 1 {
                let next_pos = match positions_iter.peek() {
                    Some(&&n) => n, // peek into next block index to find value of skip instruction
                    None => return Err(err::WTF),
                };
                block.skip_to_block(next_pos)?; // modifies the block so that the index of the next block is sorrect
            } // modify every block except the last
            self.blocks.0[*pos - 1] = *block; // insert block into the correct position in block array
        }
        self.metadata.title(song, title); // set title
        Ok(song)
    }

    /// Returns all bytes in this save file as a `Vec<u8>`.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SAVE_SIZE);
        for b in self.sram.data.iter() {
            out.push(*b);
        }
        for b in self.metadata.bytes().iter() {
            out.push(*b);
        }
        for block in self.blocks.0.iter() {
            for b in block.data.iter() {
                out.push(*b);
            }
        }
        out
    }
}

struct LsdjBlockTable([LsdjBlock; BLOCK_COUNT]); // must be wrapped in a struct to allow implementation

impl LsdjBlockTable {
    fn fill(&mut self, savefile: &mut File) -> io::Result<()> {
        savefile.seek(Start(BLOCK_ADDRESS))?;
        for block in self.0.iter_mut() {
            savefile.take(BLOCK_SIZE as u64).read(&mut block.data)?;
        }
        Ok(())
    }

    fn from(mut savefile: &mut File) -> io::Result<LsdjBlockTable> {
        let mut table = LsdjBlockTable([LsdjBlock::empty(); BLOCK_COUNT]);
        table.fill(&mut savefile)?;
        Ok(table)
    }
}

impl fmt::Debug for LsdjSram {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "     | ")?;
        for i in 0..0x10 {
            write!(f, "{:X} | ", i)?;
        }
        write!(f, "\n")?;
        for disp in 0..(SRAM_SIZE / 0x10) {
            write!(f, "{:04X}  | ", disp * 0x10)?;
            for offset in 0..0x10 {
                write!(f, "{:02X}| ", self.data[disp * 0x10 + offset])?;
            }
            write!(f, "\n")?;
        }
        Ok(())
    }
}

impl fmt::Debug for LsdjSave {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SRAM: {:?}", self.sram)?;
        write!(f, "metadata: {:?}", self.metadata)?;
        write!(f, "blocks:\n")?;
        for (i, block) in self.blocks.0.iter().enumerate() {
            write!(f, "block {:X}: {:?}", i + 1, block)?;
        }
        Ok(())
    }
}

impl PartialEq for LsdjSram {
    fn eq(&self, rhs: &Self) -> bool {
        self.data.iter().zip(rhs.data.iter()).all(|(a, b)| a == b)
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::fs::File;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_lsdjsave_load() -> io::Result<()> {
        let savepath = PathBuf::from("saves/test.sav");
        let mut savefile = File::open(savepath)?;
        let save = LsdjSave::from(&mut savefile)?;
        println!("{:?}", save);
        Ok(())
    }

    #[test]
    fn print_export_song() -> io::Result<()> {
        let savepath = PathBuf::from("saves/test.sav");
        let mut savefile = File::open(savepath)?;
        let save = LsdjSave::from(&mut savefile)?;
        let bytes = save.export_song(0);
        println!("{:02X?}", bytes);
        Ok(())
    }

    #[test]
    fn test_export_song() {
        let save = LsdjSave::empty();
        let bytes = save.export_song(0);
        assert_eq!(bytes, vec![]); // should be empty, as song 0 does not exist
    }

    #[test]
    fn test_import_song() {
        let mut save = LsdjSave::empty();
        for block in save.metadata.alloc_table.iter_mut() {
            *block = 0;
        }
        let bytes = vec![1, 2, 3];
        let song = save.import_song(&bytes, [0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(song, Err(err::SONGS_FULL));
        let mut block_bytes = vec![5; BLOCK_SIZE * 3];
        block_bytes[BLOCK_SIZE - 2] = 0xe0;
        block_bytes[BLOCK_SIZE - 1] = b'x';
        block_bytes[BLOCK_SIZE * 2 - 2] = 0xe0;
        block_bytes[BLOCK_SIZE * 2 - 1] = b'x';
        block_bytes[BLOCK_SIZE * 3 - 2] = 0xe0;
        block_bytes[BLOCK_SIZE * 3 - 1] = 0xff;
        let mut empty_save = LsdjSave::empty();
        let title = [b'T', b'E', b'S', b'T', 0, 0, 0, 0];
        assert_eq!(empty_save.import_song(&block_bytes, title), Ok(0));
        println!("{:?}", empty_save);
    }

    #[test]
    fn test_lsdjsram_partialeq() {
        let sram = LsdjSram::empty();
        let eq_sram0 = LsdjSram {
            position: 0,
            data: [0; SRAM_SIZE]
        };
        let neq_sram = LsdjSram {
            position: 0,
            data: [1; SRAM_SIZE]
        };
        let eq_sram1 = LsdjSram {
            position: 1234,
            data: [0; SRAM_SIZE]
        };
        assert!(sram == eq_sram0);
        assert!(sram != neq_sram);
        assert!(sram == eq_sram1);
    }
}
