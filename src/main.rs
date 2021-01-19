use std::io;
use std::fs::File;
use std::path::PathBuf;

use structopt::StructOpt;

use lsdj::LsdjSave;
use lsdj::LsdjBlockExt;

macro_rules! or_die {
    ($e:expr) => {
        if let Err(e) = $e {
            eprintln!("{:?}", e);
            std::process::exit(1);
        }
    };
}

const ERR_COMPRESSION: &str = "SRAM compression failed";
const ERR_TITLE_FMT: &str   = "Title incorrectly formatted";

#[derive(StructOpt, Debug)]
#[structopt(name = "lsdjtool")]
struct Opt {
    /// List indices, titles, and versions of songs present in save file
    #[structopt(short, long, conflicts_with_all(&["export", "import-from"]))]
    list_songs: bool,

    /// Index of song to be exported from save file
    #[structopt(short, long, value_name("INDEX"), conflicts_with("import-from"))]
    export: Option<u8>,

    /// Export working song (SRAM)
    #[structopt(short = "x", long = "export-sram", conflicts_with_all(&["export", "import-from"]))]
    export_sram: bool,

    /// File from which to import blocks of compressed song data
    #[structopt(short, long, value_name("SONGFILE"), parse(from_os_str))]
    import_from: Option<PathBuf>,

    /// Title for imported song (at most eight characters, uppercase alphanumeric ASCII plus space
    /// (0x20),
    /// lowercase 'x' represents the lightning bolt character). Defaults to
    /// SONGNAME.
    #[structopt(short, long, value_name("TITLE"), requires("import-from"))]
    title: Option<String>,

    /// Output file (defaults to stdout)
    #[structopt(short, long, value_name("OUTFILE"), parse(from_os_str))]
    output: Option<PathBuf>,

    /// Save file to read from
    #[structopt(value_name("SAVEFILE"), parse(from_os_str))]
    savefile: PathBuf,
}

fn main() -> io::Result<()> {
    let opt = Opt::from_args();
    let mut savefile = File::open(opt.savefile)?;
    let mut outfile: Box<dyn io::Write> = match opt.output {
        Some(path) => Box::new(File::create(path)?),
        None => Box::new(io::stdout()),
    };
    let save = LsdjSave::from(&mut savefile)?;
    if opt.list_songs {
        let songlist = save.metadata.list_songs();
        outfile.write_all(songlist.as_bytes())?;
        return Ok(());
    } else if opt.export_sram {
        let mut save_copy = save;
        let mut blocks = Vec::new();
        save_copy.compress_sram_into(&mut blocks, 1).expect(ERR_COMPRESSION);
        let bytes = blocks.bytes();
        outfile.write_all(&bytes)?;
        return Ok(())
    } else if opt.export != None {
        let index = opt.export.unwrap();
        let song_bytes = save.export_song(index);
        outfile.write_all(&song_bytes)?;
        return Ok(())
    } else if opt.import_from != None {
        let blockpath = opt.import_from.unwrap();
        let mut blockfile = File::open(blockpath)?;

        let mut bytes = Vec::new(); // bytes of compressed song data
        lsdj::read_blocks_from_file(&mut blockfile, &mut bytes)?;
        let mut outsave = save;

        let title_result = match opt.title {
            Some(t) => lsdj::lsdjtitle_from(t),
            None => lsdj::lsdjtitle_from("SONGNAME"),
        };
        let title = title_result.expect(ERR_TITLE_FMT);
        or_die!(outsave.import_song(&bytes, title));
        let save_bytes = outsave.bytes();
        outfile.write_all(&save_bytes)?;
        return Ok(());
    }
    Ok(())
}
