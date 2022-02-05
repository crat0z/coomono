extern crate onig;
extern crate select;

use colored::*;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
// use number_prefix::NumberPrefix;
use onig::*;
use reqwest::Client;
use select::document::Document;
use select::predicate::{Class, Name, Predicate};
use spinners::{Spinner, Spinners};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
pub struct Creator {
    options: DownloadOptions,
    client: Client,
    user: String,
    service: String,
    url: String,
    base_url: String,
    posts: Vec<Post>,
}

pub struct DownloadOptions {
    pub url: String,
    pub proxy: Option<String>,
    pub sorted: bool,
}

struct Post {
    title: String,
    attachments: Vec<String>,
    images: Vec<String>,
}

impl Creator {
    pub fn new(options: DownloadOptions) -> Option<Creator> {
        let regex =
            Regex::new("^(https?://)?(www.)?(coomer.party|kemono.party)/(.+)/user/(.+)$").unwrap();

        if regex.is_match(&options.url) {
            let matches = regex.captures(&options.url).unwrap();

            let base_url = format!("https://{}", matches.at(3).unwrap());
            let service = matches.at(4).unwrap().to_string();
            let user = matches.at(5).unwrap().to_string();
            let url = format!("{}/{}/user/{}", base_url, service, user);

            println!("URL: {}", url.bold());

            let mut cb = reqwest::ClientBuilder::new();

            if let Some(ref s) = options.proxy {
                info!("using proxy: {}", s.bold());
                let p = reqwest::Proxy::all(s).expect("invalid proxy");
                cb = cb.proxy(p);
            }

            Some(Creator {
                options,
                client: cb.build().unwrap(),
                user,
                service,
                url,
                base_url,
                posts: Vec::new(),
            })
        } else {
            None
        }
    }

    async fn collect_posts(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let response = self.client.get(&self.url).send().await?.text().await?;

        let doc = Document::from(response.as_str());

        let mut post_urls = Vec::new();
        let mut pages = Vec::new();

        // find all the other pages
        doc.find(Name("li").descendant(Name("a")))
            .filter_map(|n| n.attr("href"))
            .for_each(|x| {
                if x.contains("?o=") && !pages.contains(&x) {
                    pages.push(x);
                    info!("found {}: {}", "page".blue(), x);
                }
            });

        let sp = Spinner::new(&Spinners::Point, "Found 0 posts".into());
        // post-card__headings contain descendants `a` with href for posts
        doc.find(Class("post-card__heading").descendant(Name("a")))
            .filter_map(|n| n.attr("href"))
            .for_each(|x| {
                post_urls.push(x.to_string());
                info!("found {}: {}", "post".green(), x);
                sp.message(format!("Found {} posts", post_urls.len()));
            });

        // collect all the posts from the other pages
        for page in pages.iter() {
            let response = self
                .client
                .get(format!("{}{}", self.base_url, page))
                .send()
                .await?
                .text()
                .await?;

            Document::from(response.as_str())
                .find(Class("post-card__heading").descendant(Name("a")))
                .filter_map(|n| n.attr("href"))
                .for_each(|x| {
                    post_urls.push(x.to_string());
                    info!("found {}: {}", "post".green(), x);
                    sp.message(format!("Found {} posts", post_urls.len()));
                });
        }
        sp.stop();

        println!();

        Ok(post_urls)
    }

    pub async fn get_all_posts(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let post_urls = self.collect_posts().await?;

        let sp = Spinner::new(&Spinners::Point, "Found 0 attachments, 0 images".into());

        let mut attachments_count = 0;
        let mut images_count = 0;

        // collect all Post items for download
        for post in post_urls.iter() {
            let request = self
                .client
                .get(format!("{}{}", self.base_url, post))
                .send()
                .await?
                .text()
                .await?;

            let mut attachments = Vec::new();
            let mut images = Vec::new();

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
            doc.find(Class("post__attachment-link"))
                .filter_map(|n| n.attr("href"))
                .for_each(|x| {
                    let s = x.to_string();
                    if !attachments.contains(&s) {
                        info!("found {}: {}", "attachment".yellow(), s);
                        attachments.push(s);
                    }
                });

            // find all images
            doc.find(Class("fileThumb"))
                .filter_map(|n| n.attr("href"))
                .for_each(|x| {
                    let s = x.to_string();
                    if !images.contains(&s) {
                        info!("found {}: {}", "image".cyan(), s);
                        images.push(s);
                    }
                });

            attachments_count += attachments.len();
            images_count += images.len();

            // insert into post collection
            self.posts.push(Post {
                title,
                attachments,
                images,
            });

            sp.message(format!(
                "Found {} attachments, {} images",
                attachments_count, images_count
            ));
        }

        sp.stop();

        println!();

        Ok(())
    }

    async fn download_url_to_file(
        &self,
        url: &str,
        base_path: &Path,
        filename: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let full_file_path = base_path.join(filename);

        if full_file_path.exists() {
            info!(
                "file {} already downloaded",
                full_file_path.to_str().unwrap()
            );
            return Ok(());
        }

        let r = self.client.get(url).send().await?;

        let size = r.content_length().unwrap();

        let pb = ProgressBar::new(size).with_message(filename.to_string());

        pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} {msg:20!.bold} {percent:>3}% [{wide_bar:.cyan/blue}] {bytes:^10!} / {total_bytes:^10!} | {bytes_per_sec:>12!}")
        .progress_chars("#>-"));

        let mut dest = File::create(full_file_path)?;

        let mut stream = r.bytes_stream();

        let mut finished = 0;

        while let Some(item) = stream.next().await {
            let chunk = item.unwrap();
            finished += chunk.len() as u64;
            dest.write_all(&chunk)?;
            pb.set_position(finished);
        }

        pb.finish();

        Ok(())
    }

    async fn prepare_download(
        &self,
        base_path: &Path,
        sub_link: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !sub_link.starts_with('/') {
            info!("file {} cannot be downloaded", sub_link.bold());
            return Ok(());
        }
        let full_url = format!("{}{}", self.base_url, sub_link);
        let split: Vec<&str> = sub_link.split("f=").collect();
        let filename = split[1];

        self.download_url_to_file(&full_url, base_path, filename)
            .await?;

        Ok(())
    }

    pub async fn download(&self) -> Result<(), Box<dyn std::error::Error>> {
        let top_dir_name = format!("{} - {}", self.service, self.user);
        let top_dir = Path::new(&top_dir_name);

        // for unsorted downloads
        let image_dir = top_dir.join("images");
        let attachments_dir = top_dir.join("attachments");

        // create our dir
        if !top_dir.is_dir() {
            fs::create_dir_all(top_dir)?;
        }

        // create images/attachment folders if they dont exist, and we're downloading unsorted
        if !self.options.sorted {
            info!("creating images/attachments directories");
            if !image_dir.is_dir() {
                fs::create_dir_all(top_dir.join("images"))?;
            }
            if !attachments_dir.is_dir() {
                fs::create_dir_all(top_dir.join("attachments"))?;
            }
        }

        for post in self.posts.iter() {
            if self.options.sorted {
                let mut post_name = post.title.clone();
                post_name = sanitize_filename::sanitize(post_name.as_str());
                post_name.truncate(90);

                let post_path = top_dir.join(post_name);

                if !post_path.is_dir() {
                    fs::create_dir_all(&post_path)?;
                    info!("created dir: {}", post_path.to_str().unwrap());
                }

                for attachment in post.attachments.iter() {
                    self.prepare_download(&post_path, attachment).await?;
                }

                for image in post.images.iter() {
                    self.prepare_download(&post_path, image).await?;
                }
            } else {
                for attachment in post.attachments.iter() {
                    self.prepare_download(&attachments_dir, attachment).await?;
                }
                for image in post.images.iter() {
                    self.prepare_download(&image_dir, image).await?;
                }
            }
        }

        Ok(())
    }
}
