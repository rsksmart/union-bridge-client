use std::collections::HashMap;
use std::time::Duration;
use std::{error, fmt};

use bitcoincore_rpc::jsonrpc::{self, Request, Response, Transport};
use reqwest::blocking::Client;

const DEFAULT_URL: &str = "http://localhost";
const DEFAULT_PORT: u16 = 8332; // the default RPC port for bitcoind.
const DEFAULT_TIMEOUT_SECONDS: u64 = 15;

/// An HTTPS transport that uses [`reqwest`] and is useful for running a bitcoind RPC client.
#[derive(Clone, Debug)]
pub struct ReqwestHttpsTransport {
    /// URL of the RPC server.
    url: String,
    /// The value of the `Authorization` HTTP header, i.e., a base64 encoding of 'user:password'.
    basic_auth: Option<String>,
    /// Headers to be added to the request.
    headers: HashMap<String, String>,
    /// HTTP client with connection pooling
    client: Client,
}

impl Default for ReqwestHttpsTransport {
    fn default() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create reqwest client");

        ReqwestHttpsTransport {
            url: format!("{}:{}", DEFAULT_URL, DEFAULT_PORT),
            basic_auth: None,
            headers: HashMap::new(),
            client,
        }
    }
}

impl ReqwestHttpsTransport {
    /// Constructs a new [`ReqwestHttpsTransport`] with default parameters.
    pub fn new() -> Self {
        ReqwestHttpsTransport::default()
    }

    /// Returns a builder for [`ReqwestHttpsTransport`].
    pub fn builder() -> Builder {
        Builder::new()
    }

    fn request<R>(&self, req: impl serde::Serialize) -> Result<R, Error>
    where
        R: for<'a> serde::de::Deserialize<'a>,
    {
        let mut request = self.client.post(&self.url).json(&req);

        if let Some(auth) = &self.basic_auth {
            request = request.header("Authorization", auth);
        }

        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        let response = request.send()?;

        if !response.status().is_success() {
            return Err(Error::Http(HttpError {
                status_code: response.status().as_u16() as i32,
                body: response.text().unwrap_or_default(),
            }));
        }

        Ok(response.json()?)
    }
}

impl Transport for ReqwestHttpsTransport {
    fn send_request(&self, req: Request) -> Result<Response, jsonrpc::Error> {
        Ok(self.request(req)?)
    }

    fn send_batch(&self, reqs: &[Request]) -> Result<Vec<Response>, jsonrpc::Error> {
        Ok(self.request(reqs)?)
    }

    fn fmt_target(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.url)
    }
}

/// Builder for simple bitcoind [`ReqwestHttpsTransport`].
#[derive(Clone, Debug)]
pub struct Builder {
    tp: ReqwestHttpsTransport,
}

impl Builder {
    /// Constructs a new [`Builder`] with default configuration and the URL to use.
    pub fn new() -> Builder {
        Builder { tp: ReqwestHttpsTransport::new() }
    }

    /// Sets the timeout after which requests will abort if they aren't finished.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.tp.client = Client::builder()
            .timeout(timeout)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .build()
            .expect("Failed to create reqwest client");
        self
    }

    /// Sets the URL of the server to the transport.
    pub fn url(mut self, url: &str) -> Result<Self, Error> {
        self.tp.url = url.to_owned();
        Ok(self)
    }

    /// Adds authentication information to the transport.
    pub fn basic_auth(mut self, user: String, pass: Option<String>) -> Self {
        let mut s = user;
        s.push(':');
        if let Some(ref pass) = pass {
            s.push_str(pass.as_ref());
        }
        self.tp.basic_auth = Some(format!("Basic {}", &base64::encode(s.as_bytes())));
        self
    }

    /// Adds Headers information to the transport.
    pub fn header(mut self, key: String, value: String) -> Self {
        self.tp.headers.insert(key, value);
        self
    }

    /// Builds the final [`ReqwestHttpsTransport`].
    pub fn build(self) -> ReqwestHttpsTransport {
        self.tp
    }
}

impl Default for Builder {
    fn default() -> Self {
        Builder::new()
    }
}

/// An HTTP error.
#[derive(Debug)]
pub struct HttpError {
    /// Status code of the error response.
    pub status_code: i32,
    /// Raw body of the error response.
    pub body: String,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "status: {}, body: {}", self.status_code, self.body)
    }
}

impl error::Error for HttpError {}

/// Error that can happen when sending requests.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// JSON parsing error.
    Json(serde_json::Error),
    /// Reqwest error.
    Reqwest(reqwest::Error),
    /// HTTP error that does not contain valid JSON as body.
    Http(HttpError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            Error::Json(ref e) => write!(f, "parsing JSON failed: {}", e),
            Error::Reqwest(ref e) => write!(f, "reqwest: {}", e),
            Error::Http(ref e) => write!(f, "http ({})", e),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        use self::Error::*;

        match *self {
            Json(ref e) => Some(e),
            Reqwest(ref e) => Some(e),
            Http(ref e) => Some(e),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Reqwest(e)
    }
}

impl From<Error> for jsonrpc::Error {
    fn from(e: Error) -> jsonrpc::Error {
        match e {
            Error::Json(e) => jsonrpc::Error::Json(e),
            e => jsonrpc::Error::Transport(Box::new(e)),
        }
    }
}
