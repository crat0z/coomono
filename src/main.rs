
extern crate colored;
extern crate indicatif;
extern crate onig;
extern crate sanitize_filename;
extern crate select;

use std::{
    path::{Path},
};

use clap::Parser;

use futures_util::{stream, StreamExt, TryFutureExt};
use onig::Regex;
use select::{
    document::Document,
    predicate::{Class, Name, Predicate},
};
use tokio::{fs::File, io::AsyncWriteExt};

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

    let regex =
        Regex::new("^(https?://)?(www.)?(coomer.party|kemono.party)/(.+)/user/(.+)$").unwrap();

    let url = args.url;

    if regex.is_match(&url) {
        let matches = regex.captures(&url).unwrap();

        let base_url = format!("https://{}", matches.at(3).unwrap());
        let service = matches.at(4).unwrap().to_string();
        let user = matches.at(5).unwrap().to_string();
        let url = format!("{}/{}/user/{}", base_url, service, user);

        let client = reqwest::Client::builder().build().unwrap();
        let response = client.get(&url).send().await?.text().await?;
        let doc = Document::from(response.as_str());

        let (s, r) = flume::unbounded::<String>();

        let mut handles = Vec::new();

        for _ in 0..8 {
            let recv = r.clone();
            let b_url = base_url.clone();
            let handle = tokio::spawn(async move {
                let client = reqwest::Client::new();
                loop {
                    match recv.recv_async().await {
                        Ok(url) => {
                            let full_url = format!("{}{}", &b_url, url);
                            let split = url.split("f=");
                            let filename = split.last().unwrap().to_string();

                            if ! Path::new(&filename).exists() {
                                let mut file = File::create(&filename).await.unwrap();

                                let mut response =
                                    client.get(&full_url).send().await.unwrap().bytes_stream();

                                while let Some(chunk) = response.next().await {
                                    let bytes = chunk.unwrap();
                                    file.write_all(&bytes).await.unwrap();
                                }

                                println!("downloaded file {}", filename);
                            }
                        }
                        Err(_) => {
                            println!("exiting...");
                            return;
                        }
                    }
                }
            });

            handles.push(handle);
        }

        let mut pages = Vec::new();
        let mut posts = Vec::new();

        // find all the other pages
        doc.find(Name("li").descendant(Name("a")))
            .filter_map(|n| n.attr("href"))
            .for_each(|x| {
                if x.contains("?o=") && !pages.contains(&x) {
                    pages.push(x);
                }
            });

        // post-card__headings contain descendants `a` with href for posts
        doc.find(Class("post-card__heading").descendant(Name("a")))
            .filter_map(|n| n.attr("href"))
            .for_each(|x| {
                posts.push(x.to_string());
            });

        // collect all the posts from the other pages
        for page in pages.iter() {
            let response = client
                .get(format!("{}{}", base_url, page))
                .send()
                .await?
                .text()
                .await?;

            Document::from(response.as_str())
                .find(Class("post-card__heading").descendant(Name("a")))
                .filter_map(|n| n.attr("href"))
                .for_each(|x| {
                    posts.push(x.to_string());
                });
        }

        // collect all Post items for download
        for post in posts.iter() {
            let request = client
                .get(format!("{}{}", base_url, post))
                .send()
                .await?
                .text()
                .await?;

            let doc = Document::from(request.as_str());

            let title_node = doc
                .find(Class("post__content").descendant(Name("pre")))
                .next();

            let title;
            match title_node {
                Some(s) => title = s.text(),
                None => title = "Untitled".to_string(),
            }

            // find all "attachments", "downloads"
            stream::iter(
                doc.find(Class("post__attachment-link"))
                    .filter_map(|n| n.attr("href")),
            )
            .for_each(|x| {
                let st = x.to_string();
                s.send_async(st).unwrap_or_else(|_| panic!("lol"))
            })
            .await;

            // find all images
            stream::iter(doc.find(Class("fileThumb")).filter_map(|n| n.attr("href")))
                .for_each(|x| {
                    let st = x.to_string();
                    s.send_async(st).unwrap_or_else(|_| panic!("lolol"))
                })
                .await;
        }

        drop(s);

        for h in handles {
            h.await?;
        }
    }

    Ok(())
}
