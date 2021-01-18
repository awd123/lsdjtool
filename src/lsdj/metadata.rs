use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom::Start;
use std::fs::File;
use std::fmt;
use std::str::from_utf8;

use crate::lsdj::err;

const TITLE_TABLE_ADDRESS  : u64   = 0x8000;
const TITLE_LENGTH         : usize = 8;
const SONG_SLOTS           : usize = 0x20;
const _TITLE_TABLE_LENGTH   : usize = TITLE_LENGTH * SONG_SLOTS;
const _VERSION_TABLE_ADDRESS: u64   = 0x8100;
const VERSION_TABLE_LENGTH : usize = 0x20;
const _EMPTY_BYTES_ADDRESS  : u64   = 0x8120;
const EMPTY_BYTES_LENGTH   : usize = 0x1e;
const _SRAM_INIT_CHK_ADDRESS: u64   = 0x813e;
const SRAM_INIT_CHK_LENGTH : usize = 2;
const _WORKING_SONG_ADDRESS : u64   = 0x8140;
const _ALLOC_TABLE_ADDRESS  : u64   = 0x8141;
const ALLOC_TABLE_LENGTH   : usize = 0xbf;

const SRAM_INIT_CHK_BYTES: [u8; 2] = [b'j', b'k'];

/// LSDj song titles consist of at most eight ASCII characters, padded with zeros.
pub type LsdjTitle = [u8; TITLE_LENGTH];

/// Contains a representation of all metadata in an LSDj save file (all data between
/// addresses `$8000` and `$81ff`).
pub struct LsdjMetadata {
    /// Contains the titles of all $20 songs on the save file.
    pub title_table  : [LsdjTitle; SONG_SLOTS],
    /// Contains the version byte of each song on the save file.
    ///
    /// The version is a one-byte number which is incremented every time a song is saved.
    pub version_table: [u8; VERSION_TABLE_LENGTH],
    /// Filled with zeros.
    pub empty_bytes  : [u8; EMPTY_BYTES_LENGTH],
    /// LSDj sets to `[$6a, $6b]` (`['j', 'k']`) on init.
    pub sram_init_chk: [u8; SRAM_INIT_CHK_LENGTH],
    /// Byte representing the index of the song currently loaded into SRAM.
    pub working_song : [u8; 1],
    /// Block allocation table, containing information about which blocks are being used.
    ///
    /// Each byte in the allocation table represents a $200-byte block of compressed song data
    /// (located between addresses `$8200` and `$1ffff` in the save file). The byte in the allocation
    /// table indicates the index of the song to which the block in question is assigned, or
    /// is set to $ff is the block is not allocated to any song.
    pub alloc_table  : [u8; ALLOC_TABLE_LENGTH],
}

/// Removes extraneous (nonsense) characters from a LittleSoundDj song title.
/// 
/// When LSDj saves songs, the song titles, if less than the eight-character limit, are sometimes
/// suffixed with random characters after their terminating null byte. This function removes
/// all bytes after a null byte is found.
/// 
/// # Example
/// ```
/// let title: LsdjTitle = [b'T', b'I', b'T', b'L', b'E', 0, b'C', b'R'];
/// assert_eq!(strip_title(title), [b'T', b'I', b'T', b'L', b'E', 0, 0, 0]);
/// ```
fn strip_title(src: LsdjTitle) -> LsdjTitle {
    let mut out = [0; TITLE_LENGTH];
    let mut end_reached = false;
    for (inc, outc) in src.iter().zip(out.iter_mut()) {
        if *inc != 0 && !end_reached {
            *outc = *inc; // move a byte from input to output if chars remain in title
        } else {
            end_reached = true; // no more characters left in title
            *outc = 0; // pad output with zeroes
        }
    }
    out
}

/// Takes an `&str` and returns an `LsdjTitle` on success, or an error if String can't
/// be converted to an LsdjTitle.
pub fn lsdjtitle_from<'a>(from: &'a str) -> Result<LsdjTitle, &'static str> {
    let mut title = [0; TITLE_LENGTH];

    if from.len() > TITLE_LENGTH {
        return Err(err::BAD_TITLE_FMT); // error if title is too long
    }
    
    for (inc, outc) in from.bytes().zip(title.iter_mut()) {
        match inc {
            b'A'..=b'Z' | b'0'..=b'9' | b'x' | b' ' => *outc = inc, // copy byte to output if valid title character
            _ => return Err(err::BAD_TITLE_FMT), // error otherwise
        }
    }

    for i in from.len()..title.len() {
        title[i] = 0; // fill rest of title with zeros
    }
    Ok(title)
}

impl LsdjMetadata {
    /// Returns an `LsdjMetadata` with all fields filled with zeros, except sram_init_chk,
    /// which is set to 'jk' and alloc_table, which is filled with $ff (which indicates
    /// an unallocated block).
    pub fn empty() -> LsdjMetadata {
        LsdjMetadata {
            title_table   : [[0; TITLE_LENGTH]; SONG_SLOTS],
            version_table : [0; VERSION_TABLE_LENGTH],
            empty_bytes   : [0; EMPTY_BYTES_LENGTH],
            sram_init_chk : SRAM_INIT_CHK_BYTES,
            working_song  : [0],
            alloc_table   : [0xff; ALLOC_TABLE_LENGTH] // unallocated blocks represented by $ff
        }
    }

    /// Populates the struct with data from the given File.
    fn fill(&mut self, savefile: &mut File) -> io::Result<()> {
        savefile.seek(Start(TITLE_TABLE_ADDRESS))?; // seek to beginning of metadata ($8000)
        for i in 0..SONG_SLOTS {
            savefile.take(TITLE_LENGTH as u64).read(&mut self.title_table[i])?; // read titles
        }
        savefile.take(VERSION_TABLE_LENGTH as u64).read(&mut self.version_table)?; // read versions
        savefile.take(EMPTY_BYTES_LENGTH as u64).read(&mut self.empty_bytes)?;
        savefile.take(SRAM_INIT_CHK_LENGTH as u64).read(&mut self.empty_bytes)?;
        savefile.take(1).read(&mut self.working_song)?;
        savefile.take(ALLOC_TABLE_LENGTH as u64).read(&mut self.alloc_table)?;
        Ok(())
    }

    /// Returns an instance of `LsdjMetadata` pre-filled with the metadata from the given File.
    pub fn from(mut savefile: &mut File) -> io::Result<LsdjMetadata> {
        let mut metadata = LsdjMetadata::empty();
        metadata.fill(&mut savefile)?;
        Ok(metadata)
    }

    /// Checks whether the SRAM initialization check bytes are equal to 'jk' (the
    /// value they are set to by LSDj on startup).
    pub fn check_sram_init(&self) -> bool {
        self.sram_init_chk == SRAM_INIT_CHK_BYTES
    }

    /// Checks whether the given block (one-indexed) is allocated to a song.
    ///
    /// Unallocated blocks are represented by $ff, so this function returns true if
    /// the block's entry in the allocation table is not equal to $ff.
    pub fn is_allocated(&self, block_index: usize) -> bool {
        self.alloc_table[block_index - 1] != 0xff // unallocated blocks are set to $ff in the allocation table (subtraction is due to blocks being one-indexed)
    }

    /// Returns the index of the next unallocated block.
    ///
    /// Note that blocks in LSDj are one-indexed (i.e., the first block of compressed
    /// song data is block 1).
    pub fn next_empty_block(&self) -> Option<usize> {
        for block in 1..=self.alloc_table.len() {
            if !self.is_allocated(block) { return Some(block); }
        }
        None
    }

    /// Reserves `block` for song `song`.
    ///
    /// Sets `block`'s entry in the allocation table to `song`.
    pub fn reserve(&mut self, block: usize, song: u8) -> Result<(), &'static str> {
        if self.alloc_table[block - 1] != 0xff {
            return Err(err::BLOCK_TAKEN);
        } else {
            self.alloc_table[block - 1] = song;
        }
        Ok(())
    }

    /// Sets the title of the given song to `title`.
    ///
    /// Note that this function does not check whether `song` already has a title,
    /// so existing titles may be overwritten.
    pub fn title(&mut self, song: u8, title: LsdjTitle) {
        self.title_table[song as usize] = title;
    }

    /// Returns the index of the next block allocated to song `song`, starting
    /// at block `skip`.
    pub fn next_block_for(&self, song: u8, skip: usize) -> Option<usize> {
        let mut left = skip;
        for (i, belongs_to) in self.alloc_table.iter().enumerate() {
            if belongs_to == &song {
                if left == 0 {
                    return Some(i + 1); // add one to block index as they are one-indexed
                } else {
                    left -= 1;
                }
            }
        }
        None
    }

    /// Returns the number of blocks allocated to `song`.
    pub fn size_of(&self, song: u8) -> usize {
        let mut size = 0;
        for belongs_to in self.alloc_table.iter() {
            if *belongs_to == song { size += 1; }
        }
        size
    }

    /// Returns the number of blocks used in the current save file.
    ///
    /// Unallocated blocks are represented in the block allocation table by the
    /// value $ff, so this function returns the number of elements in the
    /// allocation table which contain that value.
    pub fn blocks_used(&self) -> usize {
        let mut used = 0;
        for belongs_to in self.alloc_table.iter() {
            if *belongs_to != 0xff { used += 1; }
        }
        used
    }

    /// Returns the next song index to which no blocks are allocated, or `None` if
    /// there are no remaining song slots.
    pub fn next_available_song(&self) -> Option<u8> {
        if self.blocks_used() as usize == ALLOC_TABLE_LENGTH { return None; }
        let mut song = 0;
        for _i in 0..SONG_SLOTS {
            for belongs_to in self.alloc_table.iter() {
                if *belongs_to == song { song += 1; break; }
            }
        } // nested loop is necessary(?) to catch out-of-order blocks
        if song as usize >= SONG_SLOTS {
            None    // if all 0x20 song slots are filled
        } else {
            Some(song)
        }
    }

    /// Returns a `std::String` containing a prettified representing all song
    /// titles in the save file, along with their indices and version bytes.
    pub fn list_songs(&self) -> String {
        let mut out = String::new();
        for (index, title) in self.title_table.iter().enumerate() {
            if title[0] == 0 { break; } // end of title table
            let stripped_title = &strip_title(*title);
            out.push_str(format!("{:02X}: {}.{:X}\n", index, match from_utf8(stripped_title) {
                Ok(t) => t,
                Err(_) => ""
            }, self.version_table[index]).as_str());
        }
        out
    }

    /// Returns all bytes in this instance as a `Vec<u8>`.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for t in self.title_table.iter() {
            for c in t.iter() {
                out.push(*c);
            }
        }
        for b in self.version_table.iter() {
            out.push(*b);
        }
        for b in self.empty_bytes.iter() {
            out.push(*b);
        }
        for b in self.sram_init_chk.iter() {
            out.push(*b);
        }
        for b in self.working_song.iter() {
            out.push(*b);
        }
        for b in self.alloc_table.iter() {
            out.push(*b);
        }
        out
    }
}

impl fmt::Debug for LsdjMetadata {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "song list [index: title.version]:\n")?;
        for (i, title) in self.title_table.iter().enumerate() {
            write!(f, "{:02X}: {:?}.{:X}\n", i, match from_utf8(&title[..]) {
                Ok(t) => t,
                Err(_) => ""
            }, self.version_table[i])?;
        }
        write!(f, "sram init check: {:X?}\t{}\n", self.sram_init_chk,
               if self.check_sram_init() { "[OK]" } else { "[FAIL]" })?;
        write!(f, "working song: {:02X} {:?}\n", self.working_song[0],
               match from_utf8(&self.title_table[self.working_song[0] as usize][0..]) {
                   Ok(t) => t, Err(_) => ""})?;
        write!(f, "block allocation table:\n")?;
        for disp in 0..(self.alloc_table.len() / 0x10) {
            write!(f, "{:02X}  | ", disp * 0x10)?;
            for offset in 0..0x10 {
                write!(f, "{:02X}| ", self.alloc_table[disp * 0x10 + offset])?;
            }
            write!(f, "\n")?;
        }
        //FIXME: ugly!
        write!(f, "B0  | ")?;
        for offset in 0xb0..0xbf {
            write!(f, "{:02X}| ", self.alloc_table[offset])?;
        }
        write!(f, "\n")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_title() {
        let title = [b'T', b'I', b'T', b'L', b'E', 0, b'C', b'R'];
        assert_eq!(strip_title(title), [b'T', b'I', b'T', b'L', b'E', 0, 0, 0]);
    }

    #[test]
    fn test_lsdjtitle_from() {
        let title = "TITLEx";
        assert_eq!(lsdjtitle_from(title), Ok([b'T', b'I', b'T', b'L', b'E', b'x', 0, 0]));
        let invalid_title1 = "SONGTITLE";
        assert_eq!(lsdjtitle_from(invalid_title1), Err(err::BAD_TITLE_FMT));
        let invalid_title2 = "title";
        assert_eq!(lsdjtitle_from(invalid_title2), Err(err::BAD_TITLE_FMT));
    }

    #[test]
    fn test_check_sram_init() {
        let mut metadata = LsdjMetadata::empty();
        assert!(metadata.check_sram_init());
        metadata.sram_init_chk = [b'j', b'l'];
        assert!(!metadata.check_sram_init());
    }

    #[test]
    fn test_is_allocated() {
        let mut metadata = LsdjMetadata::empty();
        metadata.alloc_table[1] = 0;
        assert!(metadata.is_allocated(2));
        assert!(!metadata.is_allocated(1));
        assert!(!metadata.is_allocated(0xbf));
    }

    #[test]
    fn test_next_empty_block() {
        let mut metadata = LsdjMetadata::empty();
        metadata.alloc_table[0] = 0;
        metadata.alloc_table[1] = 0;
        metadata.alloc_table[2] = 0;
        metadata.alloc_table[3] = 0;
        assert_eq!(metadata.next_empty_block(), Some(5));
        metadata.alloc_table[2] = 0xff;
        assert_eq!(metadata.next_empty_block(), Some(3));
        metadata.alloc_table = [0; ALLOC_TABLE_LENGTH];
        assert_eq!(metadata.next_empty_block(), None);
    }

    #[test]
    fn test_reserve() -> Result<(), &'static str> {
        let mut metadata = LsdjMetadata::empty();
        assert_eq!(metadata.blocks_used(), 0);
        let song = match metadata.next_available_song() {
            Some(s) => s,
            None => return Err(err::SONGS_FULL)
        };
        while let Some(next_block) = metadata.next_empty_block() {
            metadata.reserve(next_block, song)?;
        }
        assert_eq!(metadata.blocks_used(), ALLOC_TABLE_LENGTH);
        Ok(())
    }

    #[test]
    fn test_next_block_for() {
        let mut metadata = LsdjMetadata::empty();
        metadata.alloc_table[0] = 0;
        metadata.alloc_table[1] = 1;
        metadata.alloc_table[2] = 0;
        metadata.alloc_table[3] = 0;
        metadata.alloc_table[9] = 1;
        metadata.alloc_table[56] = 3;
        metadata.alloc_table[66] = 3;
        assert_eq!(metadata.next_block_for(0, 0), Some(1));
        assert_eq!(metadata.next_block_for(1, 0), Some(2));
        assert_eq!(metadata.next_block_for(2, 0), None);
        assert_eq!(metadata.next_block_for(0, 1), Some(3));
        assert_eq!(metadata.next_block_for(0, 2), Some(4));
        assert_eq!(metadata.next_block_for(1, 1), Some(10));
        assert_eq!(metadata.next_block_for(3, 0), Some(57));
        assert_eq!(metadata.next_block_for(3, 1), Some(67));
    }

    #[test]
    fn test_size_of() {
        let mut metadata = LsdjMetadata::empty();
        for i in 0..0x10 {
            metadata.alloc_table[i] = 0;
        }
        metadata.alloc_table[0x10] = 1;
        metadata.alloc_table[0x11] = 1;
        metadata.alloc_table[0x12] = 0;
        assert_eq!(metadata.size_of(0), 17);
        assert_eq!(metadata.size_of(1), 2);
        assert_eq!(metadata.size_of(2), 0);
    }

    #[test]
    fn test_blocks_used() {
        let metadata = LsdjMetadata::empty();
        assert_eq!(metadata.blocks_used(), 0);
    }

    #[test]
    fn test_next_available_song() {
        let mut metadata = LsdjMetadata::empty();
        for i in 0..8 {
            metadata.alloc_table[i] = 0;
        }
        metadata.alloc_table[8] = 1;
        metadata.alloc_table[9] = 2;
        metadata.alloc_table[10] = 3;
        metadata.alloc_table[11] = 4;
        metadata.alloc_table[12] = 6;
        metadata.alloc_table[13] = 5;
        assert_eq!(metadata.next_available_song(), Some(7));
        let mut metadata0 = LsdjMetadata::empty();
        metadata0.alloc_table = [0; ALLOC_TABLE_LENGTH];
        assert_eq!(metadata0.next_available_song(), None);
    }
}
