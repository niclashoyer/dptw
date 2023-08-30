use chrono::prelude::*;
use clap::Parser;
use feed_rs::parser;
use html5ever::interface::TreeSink;
use indoc::indoc;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::BufWriter;
use tera::Context;
use tera::Tera;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// config file to use (toml format)
    #[arg(short, long, default_value = "config.toml")]
    config: String,
    /// parse entries that are at minimum "min" days old
    #[arg(short, long, default_value = "1")]
    min: u64,
    /// parse entries that are at maximum "max" days old
    #[arg(short, long, default_value = "1")]
    max: u64,
}

#[derive(Debug, Serialize)]
struct ArticleTag {
    pub tag: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct ConfigFeed {
    title: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct Config {
    feeds: Vec<ConfigFeed>,
}

fn gen_tagesschau(
    feeds: Vec<ConfigFeed>,
    from: chrono::DateTime<Local>,
    to: chrono::DateTime<Local>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = epub_builder::EpubBuilder::new(epub_builder::ZipLibrary::new()?)?;
    builder
        .metadata("author", "Tagesschau")?
        .metadata(
            "title",
            format!(
                "Nachrichten von {} bis {}",
                from.format("%d.%m.%Y %H:%M"),
                to.format("%d.%m.%Y %H:%M")
            ),
        )?
        .epub_version(epub_builder::EpubVersion::V20);

    for (i, feed_config) in feeds.iter().enumerate() {
        let xml = reqwest::blocking::get(feed_config.url.clone())?.text()?;
        let feed = parser::parse(xml.as_bytes())?;

        let mut context = Context::new();
        context.insert("title", &feed_config.title);
        let html = Tera::one_off(
            indoc! { r#"
                        <?xml version="1.0" encoding="UTF-8"?>
                        <html xmlns="http://www.w3.org/1999/xhtml" xml:lang="en" lang="en">
                        <head>
                        </head>
                        <body>
                            <h1>{{ title }}</h1>
                        </body>
                        </html>
                    "# },
            &context,
            true,
        )?;

        builder.add_content(
            epub_builder::EpubContent::new(format!("chapter_{}.xhtml", i), html.as_bytes())
                .title(feed_config.title.clone()),
        )?;

        for (j, entry) in feed.entries.iter().enumerate() {
            let mut elements = vec![];
            let href = entry.links.first().unwrap().href.clone();
            if let Some(p) = entry.published {
                if p < from || p > to {
                    continue;
                }
            }
            println!("{}", href);

            let html = reqwest::blocking::get(&href)?.text()?;
            let mut document = Html::parse_document(&html);

            // FIXME: don't remove .infoxbox and style it instead
            let sel_remove = &Selector::parse(".copytext-element-wrapper, .meldungsfooter, .copytext__video, .external-embed__placeholder, .infobox")?;
            let nodes_to_remove: Vec<ego_tree::NodeId> =
                document.select(sel_remove).map(|r| r.id()).collect();
            for n in nodes_to_remove {
                document.remove_from_parent(&n);
            }

            let article = {
                let sel_article = &Selector::parse("article")?;
                let mut sel = document.select(sel_article);
                if let Some(article) = sel.next() {
                    article
                } else {
                    continue;
                }
            };

            let sel_content = &Selector::parse("h1, h2, h3, p")?;
            let content = article.select(sel_content);

            for el in content {
                let node = el.value().name();
                let tag = node.to_lowercase();
                let text = el.text().fold("".to_string(), |x, s| x + s);
                elements.push(ArticleTag { tag, text });
            }

            if elements.len() > 0 {
                let mut context = Context::new();
                context.insert("elements", &elements);
                let html = Tera::one_off(
                    indoc! { r#"
                        <?xml version="1.0" encoding="UTF-8"?>
                        <html xmlns="http://www.w3.org/1999/xhtml" xml:lang="en" lang="en">
                        <head>
                        </head>
                        <body>
                            {% for el in elements %}
                            <{{ el.tag }}>{{ el.text }}</{{ el.tag }}>
                            {% endfor %}
                        </body>
                        </html>
                    "# },
                    &context,
                    true,
                )?;

                builder.add_content(
                    epub_builder::EpubContent::new(
                        format!("chapter_{}_{}.xhtml", i, j),
                        html.as_bytes(),
                    )
                    .level(2)
                    .title(entry.title.as_ref().unwrap().content.clone()),
                )?;
            }
        }
    }

    let mut output = BufWriter::new(File::create("tagesschau.epub")?);
    builder.inline_toc();
    builder.generate(&mut output)?;

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    dbg!(args.clone());

    let now: DateTime<Local> = Local::now();
    let from = now.checked_sub_days(chrono::Days::new(args.min)).unwrap();
    let to = now
        .checked_sub_days(chrono::Days::new(if args.min > 0 {
            args.min - 1
        } else {
            0
        }))
        .unwrap();

    let config: Config = toml::from_str(&fs::read_to_string(args.config)?)?;

    gen_tagesschau(config.feeds, from, to)?;
    Ok(())
}
