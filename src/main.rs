use std::path::PathBuf;
use std::io;
use std::fs::File;

use structopt::StructOpt;

use lsdj::LsdjSave;

mod lsdj;

#[derive(StructOpt, Debug)]
#[structopt(name = "lsdjtool")]
struct Opt {
    /// List indices, titles, and versions of songs present in save file
    #[structopt(short, long, conflicts_with_all(&["export", "import-from"]))]
    list_songs: bool,

    /// Index of song to be exported from save file
    #[structopt(short, long, value_name("INDEX"), conflicts_with("import-from"))]
    export: Option<u8>,

    /// File from which to import blocks of compressed song data
    #[structopt(short, long, value_name("SONGFILE"), parse(from_os_str))]
    import_from: Option<PathBuf>,

    /// Title for imported song (at most eight characters, uppercase ASCII,
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
    } else if opt.export != None {
        let index = match opt.export {
            Some(i) => i,
            None => return Ok(()),
        };
        let song_bytes = save.export_song(index);
        outfile.write_all(&song_bytes)?;
        return Ok(())
    } else if opt.import_from != None {
        let blockpath = match opt.import_from {
            Some(p) => p,
            None => PathBuf::from(""),
        };
        let mut blockfile = File::open(blockpath)?;
        let mut bytes = Vec::new(); // bytes of compressed song data
        lsdj::read_blocks_from_file(&mut blockfile, &mut bytes)?;
        let mut outsave = save;
        let title_result = match opt.title {
            Some(t) => lsdj::metadata::lsdjtitle_from(t.as_str()),
            None => lsdj::metadata::lsdjtitle_from("SONGNAME"),
        };
        match title_result {
            Ok(title) => {
                match outsave.import_song(&bytes, title) {
                    Ok(_) => (),
                    Err(e) => { eprintln!("{}", e); return Ok(()); },
                }
                println!("{:?}", outsave);
            },
            Err(e) => {
                eprintln!("{}", e);
                return Ok(());
            },
        }
    }
    Ok(())
}
