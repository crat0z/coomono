extern crate onig;
extern crate select;

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use colored::*;
use futures_util::StreamExt;
use log::info;

use onig::*;
use reqwest::Client;
use select::document::Document;
use select::predicate::{Class, Name, Predicate};
pub struct Creator {
    client: Client,
    user: String,
    service: String,
    url: String,
    base_url: String,
    posts: Vec<Post>,
}

struct Post {
    title: String,
    attachments: Vec<String>,
    images: Vec<String>,
}

impl Creator {
    pub fn new(url: String, proxy: Option<String>) -> Option<Creator> {
        let regex =
            Regex::new("^(https?://)?(www.)?(coomer.party|kemono.party)/(.+)/user/(.+)$").unwrap();

        if regex.is_match(&url) {
            let matches = regex.captures(&url).unwrap();

            let base_url = format!("https://{}", matches.at(3).unwrap());
            let service = matches.at(4).unwrap().to_string();
            let user = matches.at(5).unwrap().to_string();
            let url = format!("{}/{}/user/{}", base_url, service, user);

            info!("parsing creator: {}", user.bold());
            info!("url: {}", url.bold());

            let mut cb = reqwest::ClientBuilder::new();

            if let Some(s) = proxy {
                info!("using proxy: {}", s.bold());
                let p = reqwest::Proxy::all(s).expect("invalid proxy");
                cb = cb.proxy(p);
            }

            Some(Creator {
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

    pub async fn collect_posts(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let response = self.client.get(&self.url).send().await?.text().await?;

        let doc = Document::from(response.as_str());

        let mut post_urls = Vec::new();
        let mut pages = Vec::new();

        // post-card__headings contain descendants `a` with href for posts
        doc.find(Class("post-card__heading").descendant(Name("a")))
            .filter_map(|n| n.attr("href"))
            .for_each(|x| {
                post_urls.push(x.to_string());
                info!("found {}: {}", "post".green(), x);
            });

        // find all the other pages
        doc.find(Name("li").descendant(Name("a")))
            .filter_map(|n| n.attr("href"))
            .for_each(|x| {
                if x.contains("?o=") && !pages.contains(&x) {
                    pages.push(x);
                    info!("found {}: {}", "page".blue(), x);
                }
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
                });
        }

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

            doc.find(Class("fileThumb"))
                .filter_map(|n| n.attr("href"))
                .for_each(|x| {
                    let s = x.to_string();
                    if !images.contains(&s) {
                        info!("found {}: {}", "image".cyan(), s);
                        images.push(s);
                    }
                });

            let p = Post {
                title,
                attachments,
                images,
            };

            self.posts.push(p);
        }

        Ok(())
    }

    async fn download_url(
        &self,
        url: &str,
        base_path: &Path,
        filename: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let full_path = base_path.join(filename);

        if full_path.exists() {
            info!("file {} already downloaded", full_path.to_str().unwrap());
            return Ok(());
        }

        let r = self.client.get(url).send().await?;

        let mut dest = File::create(full_path)?;

        let mut stream = r.bytes_stream();

        while let Some(item) = stream.next().await {
            let chunk = item;
            dest.write_all(&chunk.unwrap())?;
        }
        Ok(())
    }

    pub async fn download(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let top_dir = Path::new(&self.user);

        // create our dir
        if !top_dir.is_dir() {
            fs::create_dir_all(top_dir)?;
        }

        for post in self.posts.iter() {
            let mut post_name = post.title.clone();
            post_name.truncate(90);
            post_name = sanitize_filename::sanitize(post_name.as_str());

            let post_path = top_dir.join(post_name);

            if !post_path.is_dir() {
                fs::create_dir_all(&post_path)?;
                info!("created dir: {}", post_path.to_str().unwrap());
            }

            for attachment in post.attachments.iter() {
                if !attachment.starts_with('/') {
                    info!("file {} cannot be downloaded", attachment.bold());
                    return Ok(());
                }
                let full_url = format!("{}{}", self.base_url, attachment);
                let split: Vec<&str> = attachment.split("f=").collect();
                let filename = split[1];
                self.download_url(&full_url, &post_path, filename).await?;
            }

            for image in post.images.iter() {
                if !image.starts_with('/') {
                    info!("file {} cannot be downloaded", image.bold());
                    continue;
                }
                let full_url = format!("{}{}", self.base_url, image);
                let split: Vec<&str> = image.split("f=").collect();
                let filename = split[1];
                self.download_url(&full_url, &post_path, filename).await?;
            }
        }

        Ok(())
    }
}
