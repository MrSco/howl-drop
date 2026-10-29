//! The sender's side of §8.1: the requests it makes, and the state machine of
//! one transfer from `/Ask` to the `/Upload` response.
//!
//! Browsing and picking a receiver are the caller's. A [`Transfer`] starts at
//! ASKING with a connection already open, takes the bytes TLS decrypted and
//! timer expiries, and says what to send and when the outcome is known.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use super::archive::compress::Compressor;
use super::http::{self, BodyDecoder, ResponseHead};
use super::messages::{AskRequest, DiscoverRequest, DiscoverResponse, Offer};
use super::plist;

/// ASK-2a: longer than the receiver's 15 s, so a slow user isn't cut off.
pub const ASK_TIMEOUT: Duration = Duration::from_secs(20);

/// How long to wait for the `/Upload` response once the body is sent.
pub const UPLOAD_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Largest response body accepted.
const MAX_RESPONSE: u64 = 1 << 20;

/// HTTP-6: the only paths a sender ever requests. There's deliberately no
/// way to name another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    Discover,
    Ask,
    Upload,
}

impl Path {
    pub fn as_str(self) -> &'static str {
        match self {
            Path::Discover => "/Discover",
            Path::Ask => "/Ask",
            Path::Upload => "/Upload",
        }
    }
}

/// HTTP-2: Finder's headers, which are all iOS 18 is known to need.
fn head(path: Path, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut headers = vec![("User-Agent", "AirDrop/1.0")];
    headers.extend_from_slice(extra);
    http::post_head(path.as_str(), &headers)
}

/// A complete request with a plist body and `Content-Length` (HTTP-2a).
fn plist_request(path: Path, value: &plist::Value) -> Vec<u8> {
    let body = plist::to_bytes(value);
    let len = body.len().to_string();
    let mut out = head(
        path,
        &[
            ("Content-Type", "application/octet-stream"),
            ("Connection", "keep-alive"),
            ("Content-Length", &len),
        ],
    );
    out.extend(body);
    out
}

#[derive(Debug, Clone)]
pub struct Config {
    pub computer_name: String,
    pub model_name: String,
    /// The service ID, reused as `SenderID` (MDNS-3).
    pub sender_id: String,
    pub bundle_id: String,
    pub ask_timeout: Duration,
}

impl Config {
    pub fn new(computer_name: impl Into<String>, sender_id: impl Into<String>) -> Self {
        Config {
            computer_name: computer_name.into(),
            model_name: "Linux".into(),
            sender_id: sender_id.into(),
            bundle_id: "com.apple.finder".into(),
            ask_timeout: ASK_TIMEOUT,
        }
    }
}

/// `POST /Discover` with an empty dictionary, which is what every sender
/// known to work with iOS 18 sends (§6.3).
pub fn discover_request() -> Vec<u8> {
    plist_request(Path::Discover, &DiscoverRequest::default().to_plist())
}

/// A receiver that answered `/Discover`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receiver {
    pub computer_name: String,
    pub model_name: Option<String>,
    pub record: Option<Vec<u8>>,
}

/// DISC-1: a receiver is listed only if it gave its name.
pub fn parse_discover_response(response: &Response) -> Option<Receiver> {
    if response.status != 200 {
        return None;
    }
    let value = plist::from_bytes(&response.body, &plist::Limits::default()).ok()?;
    let r = DiscoverResponse::from_plist(&value).ok()?;
    Some(Receiver {
        computer_name: r.receiver_computer_name?,
        model_name: r.receiver_model_name,
        record: r.receiver_record,
    })
}

/// The `/Ask` for an offer, in the iOS 18 shape (ASK-7).
pub fn ask_request(config: &Config, transfer_id: &str, offer: &Offer) -> Vec<u8> {
    let request = AskRequest {
        transfer_id: Some(transfer_id.into()),
        transfer_type: None,
        sender_computer_name: config.computer_name.clone(),
        sender_model_name: config.model_name.clone(),
        sender_id: config.sender_id.clone(),
        bundle_id: config.bundle_id.clone(),
        sender_record: None,
        file_icon: None,
        offer: offer.clone(),
        described: String::new(),
    };
    plist_request(Path::Ask, &request.to_plist())
}

/// The `/Upload` head for a body of `len` bytes (UP-1a, UP-9, UP-10).
pub fn upload_head(transfer_id: &str, compressor: Compressor, len: u64) -> Vec<u8> {
    let len = len.to_string();
    head(
        Path::Upload,
        &[
            ("TransferID", transfer_id),
            ("TotalBytes", "0"),
            ("Content-Type", compressor.content_type()),
            ("Connection", "keep-alive"),
            ("Content-Length", &len),
        ],
    )
}

/// A UTI for a file name, from its extension (`FileType`). `public.data`
/// when unknown.
pub fn uti(name: &str, is_directory: bool) -> &'static str {
    if is_directory {
        return "public.folder";
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "public.jpeg",
        "png" => "public.png",
        "gif" => "com.compuserve.gif",
        "heic" => "public.heic",
        "heif" => "public.heif",
        "tif" | "tiff" => "public.tiff",
        "webp" => "org.webmproject.webp",
        "mov" => "com.apple.quicktime-movie",
        "mp4" => "public.mpeg-4",
        "m4a" => "com.apple.m4a-audio",
        "mp3" => "public.mp3",
        "wav" => "com.microsoft.waveform-audio",
        "pdf" => "com.adobe.pdf",
        "txt" | "text" => "public.plain-text",
        "md" => "net.daringfireball.markdown",
        "html" | "htm" => "public.html",
        "rtf" => "public.rtf",
        "vcf" => "public.vcard",
        "ics" => "com.apple.ical.ics",
        "zip" => "public.zip-archive",
        "json" => "public.json",
        "csv" => "public.comma-separated-values-text",
        "doc" => "com.microsoft.word.doc",
        "docx" => "org.openxmlformats.wordprocessingml.document",
        "xls" => "com.microsoft.excel.xls",
        "xlsx" => "org.openxmlformats.spreadsheetml.sheet",
        "ppt" => "com.microsoft.powerpoint.ppt",
        "pptx" => "org.openxmlformats.presentationml.presentation",
        "key" => "com.apple.keynote.key",
        "pages" => "com.apple.iwork.pages.sffpages",
        "numbers" => "com.apple.iwork.numbers.sffnumbers",
        _ => "public.data",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub head: ResponseHead,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseError {
    Http(http::Error),
    /// The connection closed before a response.
    Closed,
}

impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResponseError::Http(e) => e.fmt(f),
            ResponseError::Closed => f.write_str("connection closed before a response"),
        }
    }
}

impl std::error::Error for ResponseError {}

/// Reads one response, skipping interim `1xx` ones.
#[derive(Debug, Default)]
pub struct ResponseReader {
    input: Vec<u8>,
    head: Option<(ResponseHead, BodyDecoder)>,
    body: Vec<u8>,
}

impl ResponseReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes; returns the response once it's complete.
    pub fn receive(&mut self, data: &[u8]) -> Result<Option<Response>, ResponseError> {
        self.input.extend_from_slice(data);
        self.advance(false)
    }

    /// The connection closed.
    pub fn receive_eof(&mut self) -> Result<Response, ResponseError> {
        match self.advance(true)? {
            Some(response) => Ok(response),
            None => Err(ResponseError::Closed),
        }
    }

    fn advance(&mut self, eof: bool) -> Result<Option<Response>, ResponseError> {
        let limits = http::Limits::default();
        loop {
            let Some((_, decoder)) = &mut self.head else {
                let Some((head, n)) =
                    http::parse_response(&self.input, &limits).map_err(ResponseError::Http)?
                else {
                    return Ok(None);
                };
                self.input.drain(..n);
                if (100..200).contains(&head.status) {
                    continue;
                }
                let decoder = BodyDecoder::new(head.body, MAX_RESPONSE, &limits)
                    .map_err(ResponseError::Http)?;
                self.head = Some((head, decoder));
                continue;
            };
            while !decoder.is_done() {
                let step = decoder.decode(&self.input).map_err(ResponseError::Http)?;
                self.body.extend_from_slice(&self.input[step.data]);
                self.input.drain(..step.consumed);
                if step.consumed == 0 {
                    if !eof {
                        return Ok(None);
                    }
                    decoder.finish().map_err(ResponseError::Http)?;
                }
            }
            let (head, _) = self.head.take().unwrap();
            return Ok(Some(Response {
                status: head.status,
                head,
                body: std::mem::take(&mut self.body),
            }));
        }
    }
}

/// How a transfer ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    /// The receiver said no, or didn't answer in time (ASK-1).
    Declined,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Transmit(Vec<u8>),
    /// Call [`Transfer::timeout`] after this long, replacing any earlier
    /// timer. `None` cancels it.
    SetTimer(Option<Duration>),
    /// The `/Ask` was accepted.
    Accepted,
    /// Send the upload: these head bytes, then the body, then call
    /// [`Transfer::upload_sent`].
    SendUpload(Vec<u8>),
    Done(Outcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Asking,
    Uploading,
    AwaitingResult,
    Done,
}

/// One transfer, from ASKING on (§8.1).
#[derive(Debug)]
pub struct Transfer {
    state: State,
    transfer_id: String,
    is_url: bool,
    upload: Option<(Compressor, u64)>,
    reader: ResponseReader,
    actions: VecDeque<Action>,
}

impl Transfer {
    /// Starts by sending the `/Ask`. `upload` is the compression and length
    /// of the body to upload once accepted, for a file share.
    pub fn new(
        config: &Config,
        transfer_id: &str,
        offer: &Offer,
        upload: Option<(Compressor, u64)>,
    ) -> Self {
        let is_url = matches!(offer, Offer::Urls(_));
        let actions = VecDeque::from([
            Action::Transmit(ask_request(config, transfer_id, offer)),
            Action::SetTimer(Some(config.ask_timeout)),
        ]);
        Transfer {
            state: State::Asking,
            transfer_id: transfer_id.into(),
            is_url,
            upload,
            reader: ResponseReader::new(),
            actions,
        }
    }

    pub fn poll(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    pub fn receive(&mut self, data: &[u8]) {
        if !matches!(
            self.state,
            State::Asking | State::Uploading | State::AwaitingResult
        ) {
            return;
        }
        match self.reader.receive(data) {
            Ok(Some(response)) => self.response(response),
            Ok(None) => {}
            Err(e) => self.finish(self.failure(e.to_string())),
        }
    }

    pub fn receive_eof(&mut self) {
        if matches!(self.state, State::Done) {
            return;
        }
        match self.reader.receive_eof() {
            Ok(response) => self.response(response),
            Err(e) => self.finish(self.failure(e.to_string())),
        }
    }

    pub fn timeout(&mut self) {
        match self.state {
            // ASK-2a: an unanswered `/Ask` is a decline.
            State::Asking => self.finish(Outcome::Declined),
            State::Uploading | State::AwaitingResult => {
                self.finish(Outcome::Failed("timed out".into()))
            }
            _ => {}
        }
    }

    /// The whole upload body has been written.
    pub fn upload_sent(&mut self) {
        if self.state == State::Uploading {
            self.state = State::AwaitingResult;
            self.actions
                .push_back(Action::SetTimer(Some(UPLOAD_RESPONSE_TIMEOUT)));
        }
    }

    /// A failure while asking is a decline, whatever caused it (ASK-1).
    fn failure(&self, reason: String) -> Outcome {
        if self.state == State::Asking {
            Outcome::Declined
        } else {
            Outcome::Failed(reason)
        }
    }

    fn response(&mut self, response: Response) {
        match self.state {
            State::Asking if response.status == 200 => {
                self.actions.push_back(Action::SetTimer(None));
                self.actions.push_back(Action::Accepted);
                if self.is_url {
                    // ASK-4: a URL share is done once accepted.
                    self.finish(Outcome::Delivered);
                } else {
                    let (compressor, len) = self.upload.expect("a file share has an upload");
                    self.state = State::Uploading;
                    self.actions.push_back(Action::SendUpload(upload_head(
                        &self.transfer_id,
                        compressor,
                        len,
                    )));
                }
            }
            State::Asking => self.finish(Outcome::Declined),
            // A receiver may answer before the body is all sent, typically
            // to refuse it.
            State::Uploading | State::AwaitingResult if response.status == 200 => {
                self.finish(Outcome::Delivered)
            }
            State::Uploading | State::AwaitingResult => self.finish(Outcome::Failed(format!(
                "upload refused with {}",
                response.status
            ))),
            _ => {}
        }
    }

    fn finish(&mut self, outcome: Outcome) {
        self.state = State::Done;
        self.actions.push_back(Action::SetTimer(None));
        self.actions.push_back(Action::Done(outcome));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::http::BodyKind;
    use crate::proto::messages::{AskFile, AskResponse};

    fn response(status: u16, body: &[u8]) -> Vec<u8> {
        http::response(status, &[], body)
    }

    fn file_offer() -> Offer {
        Offer::Files(vec![AskFile {
            name: "a.txt".into(),
            file_type: "public.plain-text".into(),
            size: Some(5),
            bom_path: "./a.txt".into(),
            is_directory: false,
        }])
    }

    fn drain(t: &mut Transfer) -> Vec<Action> {
        std::iter::from_fn(|| t.poll()).collect()
    }

    // spec: HTTP-6
    #[test]
    fn requests_only_known_paths() {
        let config = Config::new("Test", "0123456789ab");
        for request in [
            discover_request(),
            ask_request(&config, "ID", &file_offer()),
            upload_head("ID", Compressor::Gzip, 0),
        ] {
            let (head, _) = http::parse_request(&request, &http::Limits::default())
                .unwrap()
                .unwrap();
            assert!(["/Discover", "/Ask", "/Upload"].contains(&head.target.as_str()));
        }
    }

    #[test]
    fn uploads_once_accepted() {
        let config = Config::new("Test", "0123456789ab");
        let mut t = Transfer::new(&config, "ID", &file_offer(), Some((Compressor::Gzip, 10)));
        let actions = drain(&mut t);
        assert!(matches!(actions[0], Action::Transmit(_)));
        assert_eq!(actions[1], Action::SetTimer(Some(ASK_TIMEOUT)));
        t.receive(&response(
            200,
            &plist::to_bytes(&AskResponse::default().to_plist()),
        ));
        let actions = drain(&mut t);
        let Some(Action::SendUpload(head)) = actions.last() else {
            panic!("{actions:?}")
        };
        let (head, _) = http::parse_request(head, &http::Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(head.headers.get("TransferID"), Some("ID"));
        assert_eq!(head.body, BodyKind::Length(10));
        t.upload_sent();
        t.receive(&response(200, b""));
        assert_eq!(
            drain(&mut t).last(),
            Some(&Action::Done(Outcome::Delivered))
        );
    }

    // spec: ASK-1
    #[test]
    fn anything_but_200_is_a_decline() {
        let config = Config::new("Test", "0123456789ab");
        for reply in [response(403, b""), response(500, b"")] {
            let mut t = Transfer::new(&config, "ID", &file_offer(), Some((Compressor::Gzip, 10)));
            drain(&mut t);
            t.receive(&reply);
            assert_eq!(drain(&mut t).last(), Some(&Action::Done(Outcome::Declined)));
        }
        let mut t = Transfer::new(&config, "ID", &file_offer(), Some((Compressor::Gzip, 10)));
        t.receive_eof();
        assert_eq!(drain(&mut t).last(), Some(&Action::Done(Outcome::Declined)));
        let mut t = Transfer::new(&config, "ID", &file_offer(), Some((Compressor::Gzip, 10)));
        t.timeout();
        assert_eq!(drain(&mut t).last(), Some(&Action::Done(Outcome::Declined)));
    }

    // spec: ASK-2a
    #[test]
    fn ask_timeout_outlasts_the_receivers() {
        assert!(ASK_TIMEOUT > crate::proto::receiver::ASK_TIMEOUT);
    }

    #[test]
    fn reads_responses_in_pieces_and_skips_continue() {
        let mut reader = ResponseReader::new();
        let mut bytes = http::CONTINUE.to_vec();
        bytes.extend(response(200, b"abc"));
        for &b in &bytes[..bytes.len() - 1] {
            assert_eq!(reader.receive(&[b]), Ok(None));
        }
        let r = reader.receive(&bytes[bytes.len() - 1..]).unwrap().unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"abc"[..]));
    }
}
