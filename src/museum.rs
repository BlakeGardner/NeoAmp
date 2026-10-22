//! The Winamp Skin Museum (<https://skins.webamp.org>): its skins in the order of its front
//! page, its search, and each skin's file.
//!
//! The museum is a volunteer project, with a GraphQL API that answers plain `GET` requests
//! without an account. NeoAmp asks for [`PAGE`] skins at a time, says who it is in every
//! request (`User-Agent: NeoAmp/<version>`, from [`crate::http`]), keeps the skins it fetched
//! in a [`Cache`] folder, and treats an answer it cannot read as no answer, so
//! that a change on the museum's side costs only the Skin Museum dialog.
//!
//! Everything here blocks; the dialog runs it on threads of its own (see `museum_window`).

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::http::{self, Timeouts, Url};
use crate::skin::MAX_WSZ_BYTES;

/// The museum's web site.
pub const SITE: &str = "https://skins.webamp.org/";
/// The museum's GraphQL API.
const API: &str = "https://api.webamp.org/graphql";
/// Skins asked for at a time.
pub const PAGE: usize = 50;
/// The longest search sent, in characters.
const MAX_QUERY_CHARS: usize = 100;
/// The most an answer may take. A page of skins is about 20 KB.
const MAX_ANSWER_BYTES: usize = 1 << 20;
/// The most the cache folder holds before the files least recently used are deleted.
pub const CACHE_BYTES: u64 = 100 << 20;
/// The longest name a skin is installed under, in characters.
const MAX_NAME_CHARS: usize = 100;

/// A skin in the museum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuseumSkin {
    /// The MD5 of its file, which the museum knows it by: 32 lowercase hex digits.
    pub md5: String,
    /// Its file's name as it was found: `Bento_Classified.wsz`.
    pub filename: String,
    /// Where its `.wsz` file is.
    pub download_url: String,
}

impl MuseumSkin {
    /// The name to list it by: the file name without `.wsz` or `.zip`.
    #[must_use]
    pub fn name(&self) -> &str {
        let name = self.filename.trim();
        for ext in [".wsz", ".zip"] {
            let Some(cut) = name.len().checked_sub(ext.len()) else {
                continue;
            };
            if cut > 0 && name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(ext) {
                return &name[..cut];
            }
        }
        name
    }

    /// The file name to install it as: its name, with the characters file systems refuse
    /// replaced, and `.wsz`.
    #[must_use]
    pub fn file_name(&self) -> String {
        let name: String = self
            .name()
            .chars()
            .take(MAX_NAME_CHARS)
            .map(|c| {
                if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        // Windows drops dots and spaces at the end of a name.
        let name = name.trim().trim_end_matches('.');
        let name = if name.is_empty() { &self.md5 } else { name };
        format!("{name}.wsz")
    }
}

/// What to list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Listing {
    /// Every skin, in the order of the museum's front page: the best known first.
    Museum,
    /// The skins a search finds, best match first, leaving out those flagged NSFW.
    Search(String),
}

impl Listing {
    /// What to list for `text` typed into a search box: the museum's own order while it is
    /// blank.
    #[must_use]
    pub fn for_query(text: &str) -> Listing {
        let mut query = String::new();
        for word in text.split_whitespace() {
            if !query.is_empty() {
                query.push(' ');
            }
            query.push_str(word);
        }
        let query: String = query.chars().take(MAX_QUERY_CHARS).collect();
        let query = query.trim_end();
        if query.is_empty() {
            Listing::Museum
        } else {
            Listing::Search(query.to_owned())
        }
    }

    /// The GraphQL query for the page starting at skin `offset`.
    fn query(&self, offset: usize) -> String {
        const FIELDS: &str = "md5 filename nsfw download_url";
        match self {
            Listing::Museum => format!(
                "{{ skins(first: {PAGE}, offset: {offset}, sort: MUSEUM) {{ nodes {{ {FIELDS} }} }} }}"
            ),
            Listing::Search(text) => format!(
                "{{ search_classic_skins(query: {}, first: {PAGE}, offset: {offset}, nsfw: false) \
                 {{ {FIELDS} }} }}",
                graphql_string(text)
            ),
        }
    }
}

/// `text` as a GraphQL string: quoted and escaped, as JSON's strings are.
fn graphql_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| String::from("\"\""))
}

/// The URL that asks `query` of the API at `api`.
fn query_url(api: &str, query: &str) -> String {
    let mut url = String::with_capacity(api.len() + 7 + 3 * query.len());
    url.push_str(api);
    url.push_str("?query=");
    for b in query.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            url.push(char::from(b));
        } else {
            let _ = write!(url, "%{b:02X}");
        }
    }
    url
}

/// One page of a listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    /// The skins on it that can be shown: those flagged NSFW, or lacking a file to fetch
    /// securely, are left out.
    pub skins: Vec<MuseumSkin>,
    /// How many skins the museum sent, shown or not; the next page starts that many on.
    pub sent: usize,
}

impl Page {
    /// Whether the listing may go on after this page.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.sent >= PAGE
    }
}

/// A GraphQL answer, as far as NeoAmp reads it. Anything else in it is ignored.
#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    data: Option<Data>,
    #[serde(default)]
    errors: Vec<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct Data {
    #[serde(default)]
    skins: Option<Nodes>,
    #[serde(default)]
    search_classic_skins: Option<Vec<Option<Node>>>,
}

#[derive(Deserialize)]
struct Nodes {
    #[serde(default)]
    nodes: Vec<Option<Node>>,
}

#[derive(Deserialize)]
struct Node {
    md5: Option<String>,
    filename: Option<String>,
    nsfw: Option<bool>,
    download_url: Option<String>,
}

impl Node {
    /// The skin, if it can be shown.
    fn into_skin(self) -> Option<MuseumSkin> {
        let https = |url: &str| {
            url.get(..8)
                .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        };
        // The MD5 names the cached files, so it must be exactly that.
        let md5 = self.md5?.to_ascii_lowercase();
        let filename = self.filename.filter(|f| !f.trim().is_empty())?;
        let download_url = self.download_url.filter(|u| https(u))?;
        let is_md5 = md5.len() == 32 && md5.bytes().all(|b| b.is_ascii_hexdigit());
        (is_md5 && self.nsfw != Some(true)).then_some(MuseumSkin {
            md5,
            filename,
            download_url,
        })
    }
}

/// Read the API's answer for a page of `listing`.
///
/// # Errors
/// Fails if the answer is not JSON of the shape expected, or holds an error and no skins.
fn parse_page(listing: &Listing, json: &[u8]) -> Result<Page> {
    let answer: Answer =
        serde_json::from_slice(json).context("the museum's answer is not what NeoAmp expects")?;
    let nodes = match (listing, answer.data) {
        (Listing::Museum, Some(Data { skins: Some(n), .. })) => n.nodes,
        (
            Listing::Search(_),
            Some(Data {
                search_classic_skins: Some(n),
                ..
            }),
        ) => n,
        _ => match answer.errors.first() {
            Some(e) => bail!("the museum says: {}", e.message),
            None => bail!("the museum's answer holds no skins"),
        },
    };
    Ok(Page {
        sent: nodes.len(),
        skins: nodes
            .into_iter()
            .flatten()
            .filter_map(Node::into_skin)
            .collect(),
    })
}

/// Fetch `url`, if what it holds is no longer than `limit` bytes.
fn fetch(url: &str, limit: usize, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    let parsed = Url::parse(url)?;
    let mut response = http::get(&parsed, Timeouts::default(), cancel)?;
    Ok(response.read_to_end_limited(limit, cancel)?)
}

/// Fetch the page of `listing` that starts at skin `offset`, giving up when `cancel` says so.
///
/// # Errors
/// Fails if the museum cannot be reached, or its answer cannot be read.
pub fn fetch_page(listing: &Listing, offset: usize, cancel: &dyn Fn() -> bool) -> Result<Page> {
    fetch_page_from(API, listing, offset, cancel)
}

fn fetch_page_from(
    api: &str,
    listing: &Listing,
    offset: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<Page> {
    let json = fetch(
        &query_url(api, &listing.query(offset)),
        MAX_ANSWER_BYTES,
        cancel,
    )?;
    parse_page(listing, &json)
}

/// The skin's `.wsz` file, and where it is kept in `cache`: from there, or else from the
/// museum.
///
/// # Errors
/// Fails if it is not cached and cannot be fetched, or cannot be kept.
pub fn download(
    skin: &MuseumSkin,
    cache: &Cache,
    cancel: &dyn Fn() -> bool,
) -> Result<(Vec<u8>, PathBuf)> {
    let name = format!("{}.wsz", skin.md5);
    if let Some(wsz) = cache.get(&name) {
        return Ok((wsz, cache.path(&name)));
    }
    let wsz = fetch(&skin.download_url, MAX_WSZ_BYTES, cancel)?;
    let path = cache.put(&name, &wsz)?;
    Ok((wsz, path))
}

/// A folder of skins fetched from the museum, so that they need not be fetched again, each
/// named `<md5>.wsz`. Reading a file marks it as used;
/// once the folder holds more than its limit, the files least recently used are deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cache {
    dir: PathBuf,
    limit: u64,
}

impl Cache {
    /// The cache in `dir`, holding at most `limit` bytes.
    #[must_use]
    pub fn new(dir: PathBuf, limit: u64) -> Cache {
        Cache { dir, limit }
    }

    /// Where file `name` is, or would be, kept.
    #[must_use]
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// File `name`, if it is kept, marked as just used.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.path(name);
        let bytes = fs::read(&path).ok()?;
        // Failing to mark it only means it is deleted sooner.
        if let Ok(f) = fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some(bytes)
    }

    /// Keep `bytes` as file `name`, then make room, and return where it is kept.
    ///
    /// # Errors
    /// Fails if the file cannot be written.
    pub fn put(&self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        static WRITES: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let path = self.path(name);
        // Written beside it first, so that a file is never read half written.
        let n = WRITES.fetch_add(1, Ordering::Relaxed);
        let part = self
            .dir
            .join(format!("{name}.{}-{n}.part", std::process::id()));
        let written = fs::write(&part, bytes).and_then(|()| fs::rename(&part, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&part);
            return Err(e).with_context(|| format!("writing {}", path.display()));
        }
        self.trim();
        Ok(path)
    }

    /// Delete the files least recently used until the rest fit within the limit.
    fn trim(&self) {
        let Ok(read) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = read
            .filter_map(Result::ok)
            .filter_map(|e| {
                let meta = e.metadata().ok().filter(fs::Metadata::is_file)?;
                Some((meta.modified().ok()?, meta.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
        if total <= self.limit {
            return;
        }
        files.sort();
        for (_, len, path) in files {
            if total <= self.limit {
                break;
            }
            match fs::remove_file(&path) {
                Ok(()) => total -= len,
                Err(e) if e.kind() == io::ErrorKind::NotFound => total -= len,
                Err(e) => log::warn!("cannot delete {}: {e}", path.display()),
            }
        }
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::http::tests::serve;
    use std::time::Duration;

    const MD5: &str = "5e4f10275dcb1fb211d4a8b4f1bda236";

    fn skin(filename: &str) -> MuseumSkin {
        MuseumSkin {
            md5: MD5.into(),
            filename: filename.into(),
            download_url: format!("https://r2.webampskins.org/skins/{MD5}.wsz"),
        }
    }

    /// A scratch folder, removed again when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("neoamp-museum-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn ok(body: &str) -> Vec<Vec<u8>> {
        vec![
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_bytes(),
        ]
    }

    #[test]
    fn names_lose_their_extension_and_install_safely() {
        assert_eq!(skin("Bento_Classified.wsz").name(), "Bento_Classified");
        assert_eq!(skin("zelda theme.ZIP").name(), "zelda theme");
        assert_eq!(skin(".wsz").name(), ".wsz");
        assert_eq!(
            skin("Bento_Classified.wsz").file_name(),
            "Bento_Classified.wsz"
        );
        assert_eq!(skin("a/b:c?.wsz").file_name(), "a_b_c_.wsz");
        assert_eq!(skin("dots... .zip").file_name(), "dots.wsz");
        assert_eq!(skin("....wsz").file_name(), format!("{MD5}.wsz"));
    }

    #[test]
    fn what_is_typed_becomes_a_listing() {
        assert_eq!(Listing::for_query("  \t "), Listing::Museum);
        assert_eq!(
            Listing::for_query("  zelda   theme "),
            Listing::Search("zelda theme".into())
        );
        let long = Listing::for_query(&"ab ".repeat(100));
        assert!(
            matches!(long, Listing::Search(q) if q.chars().count() <= MAX_QUERY_CHARS && !q.ends_with(' '))
        );
    }

    #[test]
    fn queries_are_escaped_and_encoded_into_the_url() {
        let museum = Listing::Museum.query(100);
        assert!(museum.contains("offset: 100") && museum.contains("sort: MUSEUM"));
        let search = Listing::Search("say \"hi\"\\".into()).query(0);
        assert!(search.contains(r#"query: "say \"hi\"\\""#), "{search}");
        assert!(search.contains("nsfw: false"));
        let url = query_url(API, "{ a b }");
        assert_eq!(url, format!("{API}?query=%7B%20a%20b%20%7D"));
        // Url::parse keeps the escapes, and finds no fragment in them.
        let parsed = Url::parse(&query_url(API, &search)).unwrap();
        assert!(
            parsed
                .target
                .starts_with("/graphql?query=%7B%20search_classic_skins")
        );
        assert!(!parsed.target.contains(['#', ' ', '"']));
    }

    #[test]
    fn pages_keep_only_skins_that_can_be_shown() {
        let node = |md5: &str, nsfw: &str, url: &str| {
            format!(r#"{{"md5":"{md5}","filename":"x.wsz","nsfw":{nsfw},"download_url":"{url}"}}"#)
        };
        let nodes = [
            node(MD5, "false", "https://d/x.wsz"),
            node(&MD5.to_uppercase(), "null", "HTTPS://d/x.wsz"),
            node(MD5, "true", "https://d/x.wsz"),
            node("not-an-md5", "false", "https://d/x.wsz"),
            node(MD5, "false", "http://d/x.wsz"),
            r#"{"md5":null}"#.to_owned(),
            "null".to_owned(),
        ]
        .join(",");
        let json = format!(r#"{{"data":{{"skins":{{"count":9,"nodes":[{nodes}]}}}}}}"#);
        let page = parse_page(&Listing::Museum, json.as_bytes()).unwrap();
        assert_eq!(page.sent, 7);
        assert!(!page.has_more());
        assert_eq!(page.skins.len(), 2);
        assert!(page.skins.iter().all(|s| s.md5 == MD5));
        let search = format!(r#"{{"data":{{"search_classic_skins":[{nodes}]}}}}"#);
        let found = parse_page(&Listing::Search("x".into()), search.as_bytes()).unwrap();
        assert_eq!(found, page);
    }

    #[test]
    fn unreadable_answers_are_errors() {
        let museum = &Listing::Museum;
        let e = parse_page(
            museum,
            br#"{"errors":[{"message":"Bad sort"}],"data":null}"#,
        );
        assert_eq!(e.unwrap_err().to_string(), "the museum says: Bad sort");
        assert!(parse_page(museum, b"<html>").is_err());
        assert!(parse_page(museum, br#"{"data":{"skins":{"nodes":"many"}}}"#).is_err());
        let wrong_listing = br#"{"data":{"search_classic_skins":[]}}"#;
        assert!(parse_page(museum, wrong_listing).is_err());
    }

    #[test]
    fn a_page_is_fetched_with_its_query() {
        let nodes = vec![
            format!(
                r#"{{"md5":"{MD5}","filename":"x.wsz","nsfw":false,"download_url":"https://d"}}"#
            );
            PAGE
        ]
        .join(",");
        let body = format!(r#"{{"data":{{"skins":{{"nodes":[{nodes}]}}}}}}"#);
        let (address, server) = serve(vec![ok(&body)], Duration::ZERO);
        let page = fetch_page_from(&format!("{address}/graphql"), &Listing::Museum, 50, &|| {
            false
        })
        .unwrap();
        assert!(page.has_more());
        assert_eq!(page.skins.len(), PAGE);
        let requests = server.join().unwrap();
        assert!(
            requests[0]
                .starts_with("GET /graphql?query=%7B%20skins%28first%3A%2050%2C%20offset%3A%2050"),
            "{}",
            requests[0]
        );
        assert!(requests[0].contains("User-Agent: NeoAmp/"));
    }

    #[test]
    fn downloads_are_kept_and_found_again() {
        let tmp = TempDir::new("download");
        let cache = Cache::new(tmp.0.clone(), CACHE_BYTES);
        let (address, server) = serve(vec![ok("WSZ")], Duration::ZERO);
        let mut s = skin("x.wsz");
        s.download_url = format!("{address}/x.wsz");
        let (wsz, path) = download(&s, &cache, &|| false).unwrap();
        assert_eq!(
            (wsz.as_slice(), path.clone()),
            (&b"WSZ"[..], tmp.0.join(format!("{MD5}.wsz")))
        );
        assert_eq!(server.join().unwrap().len(), 1);
        // The server has gone; it comes from the cache now.
        assert_eq!(
            download(&s, &cache, &|| false).unwrap(),
            (b"WSZ".to_vec(), path)
        );
    }

    #[test]
    fn the_cache_deletes_what_was_used_longest_ago() {
        let tmp = TempDir::new("trim");
        let cache = Cache::new(tmp.0.clone(), 25);
        let hour = Duration::from_secs(3600);
        let age = |name: &str, hours: u32| {
            let f = fs::File::options()
                .write(true)
                .open(cache.path(name))
                .unwrap();
            f.set_modified(SystemTime::now() - hour * hours).unwrap();
        };
        cache.put("a", &[0; 10]).unwrap();
        cache.put("b", &[0; 10]).unwrap();
        age("a", 3);
        age("b", 2);
        // Reading "a" makes it the most recently used.
        assert!(cache.get("a").is_some());
        cache.put("c", &[0; 10]).unwrap();
        assert!(cache.get("b").is_none(), "used longest ago");
        assert!(cache.get("a").is_some() && cache.get("c").is_some());
        assert_eq!(cache.get("missing"), None);
        let leftovers: Vec<_> = fs::read_dir(&tmp.0)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(leftovers.len(), 2, "no part-written files left behind");
    }
}
