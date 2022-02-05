mod creator;

extern crate colored;
extern crate indicatif;
extern crate onig;
extern crate sanitize_filename;
extern crate select;

use clap::Parser;
use creator::{Creator, DownloadOptions};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// http(s)/socks proxy to use
    #[clap(short, long)]
    proxy: Option<String>,
    /// verbose (info) print
    #[clap(short)]
    verbose: bool,
    /// sort downloads into separate folders for each post
    #[clap(short)]
    sorted: bool,
    /// creator's URL
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.verbose {
        simple_logger::init_with_level(log::Level::Info).unwrap();
    }

    let options = DownloadOptions {
        url: args.url,
        proxy: args.proxy,
        sorted: args.sorted,
    };

    if let Some(mut c) = Creator::new(options) {
        c.get_all_posts().await?;
        c.download().await?;
    }

    Ok(())
}
