mod creator;

extern crate colored;
extern crate onig;
extern crate sanitize_filename;
extern crate select;
use clap::Parser;
use creator::Creator;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long)]
    proxy: Option<String>,
    #[clap(short)]
    debug: bool,
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.debug {
        simple_logger::init_with_level(log::Level::Debug).unwrap();
    } else {
        simple_logger::init_with_level(log::Level::Info).unwrap();
    }

    if let Some(mut c) = Creator::new(args.url, args.proxy) {
        c.collect_posts().await?;
        c.download().await?;
    }

    Ok(())
}
