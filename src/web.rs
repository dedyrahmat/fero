use crate::{Command, icons::BRAND_SVG, preferences::Accent};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose};
use fero::Transfer;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header,
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use ring::{digest, rand};
use rusqlite::{Connection, OptionalExtension, params};
use rustls::ServerConfig;
use std::{
    convert::Infallible,
    fs::File,
    io::BufReader,
    net::{IpAddr, SocketAddr, TcpListener},
    path::PathBuf,
    sync::{Arc, RwLock, mpsc::Sender},
    thread,
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::sync::{Semaphore, oneshot};
use tokio_rustls::TlsAcceptor;

const MAX_TORRENT: usize = 10_000_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct WebSettings {
    pub address: SocketAddr,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
}

impl WebSettings {
    fn tls(&self) -> Result<Arc<ServerConfig>> {
        let lan = match self.address.ip() {
            IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
            IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
        };
        ensure!(
            lan && self.address.port() != 0,
            "Enter a private LAN IP address and port"
        );
        let mut certificate =
            BufReader::new(File::open(&self.certificate).context("Open TLS certificate")?);
        let certificates =
            rustls_pemfile::certs(&mut certificate).collect::<std::io::Result<_>>()?;
        let mut private_key =
            BufReader::new(File::open(&self.private_key).context("Open TLS private key")?);
        let key = rustls_pemfile::private_key(&mut private_key)?
            .context("No supported key in the PEM file")?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Ok(Arc::new(
            ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_single_cert(certificates, key)
                .context("Load HTTPS certificate and matching key")?,
        ))
    }
}

pub enum WebAction {
    List,
    Magnet(String),
    Torrent(Vec<u8>),
    Pause(i64, bool),
}

pub type Reply = oneshot::Sender<Result<(Vec<Transfer>, Accent, bool), String>>;

#[derive(Default)]
pub struct WebControl {
    settings: Option<WebSettings>,
    server: Option<Server>,
}

#[derive(Clone)]
struct Access {
    tls: Arc<ServerConfig>,
    credentials: [u8; 32],
}

type SharedAccess = Arc<RwLock<Access>>;

impl WebControl {
    pub fn load(&mut self, db: &Connection, commands: &Sender<Command>) -> Result<()> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS web_access (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                address TEXT NOT NULL, certificate TEXT NOT NULL,
                private_key TEXT NOT NULL, credentials BLOB NOT NULL
            );",
        )?;
        let saved = db
            .query_row(
                "SELECT address, certificate, private_key, credentials FROM web_access WHERE id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, [u8; 32]>(3)?)),
            )
            .optional()?;
        if let Some((address, certificate, private_key, credentials)) = saved {
            let settings = WebSettings {
                address: address.parse()?,
                certificate: certificate.into(),
                private_key: private_key.into(),
            };
            self.settings = Some(settings.clone());
            let access = Access {
                tls: settings.tls()?,
                credentials,
            };
            self.server = Some(Server::start(settings.address, access, commands.clone())?);
        }
        Ok(())
    }

    pub fn retry_pending(&mut self, db: &Connection, commands: &Sender<Command>) -> bool {
        self.settings.is_some()
            && self.server.is_none()
            && self.load(db, commands).is_ok()
            && self.server.is_some()
    }

    pub fn view(&self) -> (Option<WebSettings>, bool) {
        (self.settings.clone(), self.server.is_some())
    }

    pub fn save(
        &mut self,
        db: &Connection,
        commands: &Sender<Command>,
        settings: Option<WebSettings>,
    ) -> Result<Option<String>> {
        let Some(settings) = settings else {
            db.execute("DELETE FROM web_access WHERE id = 1", [])?;
            self.server = None;
            self.settings = None;
            return Ok(None);
        };
        let tls = settings.tls()?;
        let secret = rand::generate::<[u8; 24]>(&rand::SystemRandom::new())
            .map_err(|_| anyhow::anyhow!("Could not generate web credentials"))?;
        let password = general_purpose::URL_SAFE_NO_PAD.encode(secret.expose());
        let access = Access {
            tls,
            credentials: hash(format!("fero:{password}").as_bytes()),
        };
        let previous = self
            .server
            .as_ref()
            .filter(|server| server.address == settings.address);
        let replacement = if previous.is_none() {
            Some(Server::start(
                settings.address,
                access.clone(),
                commands.clone(),
            )?)
        } else {
            None
        };
        db.execute(
            "REPLACE INTO web_access VALUES (1, ?1, ?2, ?3, ?4)",
            params![
                settings.address.to_string(),
                settings
                    .certificate
                    .to_str()
                    .context("Certificate path must be valid UTF-8")?,
                settings
                    .private_key
                    .to_str()
                    .context("Private key path must be valid UTF-8")?,
                access.credentials,
            ],
        )?;
        if let Some(previous) = previous {
            *previous.access.write().expect("Web credentials lock") = access;
        } else {
            self.server = replacement;
        }
        self.settings = Some(settings);
        Ok(Some(password))
    }
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    digest::digest(&digest::SHA256, bytes)
        .as_ref()
        .try_into()
        .expect("SHA-256 length")
}

struct Server {
    address: SocketAddr,
    access: SharedAccess,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(address: SocketAddr, access: Access, commands: Sender<Command>) -> Result<Self> {
        let listener =
            TcpListener::bind(address).with_context(|| format!("Listen on {address}"))?;
        listener.set_nonblocking(true)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let listener = {
            let _entered = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };
        let access = Arc::new(RwLock::new(access));
        let thread_access = access.clone();
        let (shutdown, mut stop) = oneshot::channel();
        let thread = thread::Builder::new().name("fero-https".into()).spawn(move || {
            runtime.block_on(async {
                let permits = Arc::new(Semaphore::new(16));
                loop {
                    let permit = tokio::select! {
                        biased;
                        _ = &mut stop => break,
                        result = permits.clone().acquire_owned() => result.expect("Server semaphore stays open"),
                    };
                    let accepted = tokio::select! {
                        biased;
                        _ = &mut stop => break,
                        result = listener.accept() => result,
                    };
                    let Ok((socket, _)) = accepted else {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    };
                    let access = thread_access.clone();
                    let commands = commands.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let _ = tokio::time::timeout(REQUEST_TIMEOUT, async move {
                            let tls = access.read().expect("Web credentials lock").tls.clone();
                            if let Ok(stream) = TlsAcceptor::from(tls).accept(socket).await {
                                let service = service_fn(move |request| {
                                    serve(request, address, access.clone(), commands.clone())
                                });
                                let _ = http1::Builder::new()
                                    .keep_alive(false)
                                    .serve_connection(TokioIo::new(stream), service)
                                    .await;
                            }
                        }).await;
                    });
                }
            });
        })?;
        Ok(Self {
            address,
            access,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown.take().expect("Web shutdown sender").send(());
        let _ = self.thread.take().expect("Web server thread").join();
    }
}

async fn serve(
    request: Request<Incoming>,
    address: SocketAddr,
    access: SharedAccess,
    commands: Sender<Command>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let hostname = if address.port() == 443 {
        match address.ip() {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        }
    } else {
        address.to_string()
    };
    let origin = format!("https://{hostname}");
    let headers = request.headers();
    let read = |name| headers.get(name).and_then(|value| value.to_str().ok());
    if read(header::HOST) != Some(hostname.as_str()) || request.uri().authority().is_some() {
        return Ok(response(
            StatusCode::BAD_REQUEST,
            "Use the configured Fero address".into(),
        ));
    }
    let presented = read(header::AUTHORIZATION)
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("basic"))
        .and_then(|(_, value)| general_purpose::STANDARD.decode(value).ok())
        .map(|value| hash(&value));
    let authorized = || {
        presented.is_some_and(|digest| {
            let access = access.read().expect("Web credentials lock");
            digest.ct_eq(&access.credentials).into()
        })
    };
    if !authorized() {
        let mut reply = response(
            StatusCode::UNAUTHORIZED,
            "Enter your Fero credentials".into(),
        );
        reply.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            r#"Basic realm="Fero""#.parse().unwrap(),
        );
        return Ok(reply);
    }

    let path = request.uri().path().to_owned();
    if request.method() == Method::GET && path == "/upload.js" {
        let mut reply = response(StatusCode::OK, include_str!("web_upload.js").into());
        reply.headers_mut().insert(
            header::CONTENT_TYPE,
            "text/javascript; charset=utf-8".parse().unwrap(),
        );
        return Ok(reply);
    }

    let action = match *request.method() {
        Method::GET if path == "/" => WebAction::List,
        Method::POST if read(header::ORIGIN) == Some(origin.as_str()) => {
            let limit = match path.as_str() {
                "/torrent" if read(header::CONTENT_TYPE) == Some("application/x-bittorrent") => {
                    MAX_TORRENT
                }
                "/magnet" | "/pause" | "/resume"
                    if read(header::CONTENT_TYPE) == Some("application/x-www-form-urlencoded") =>
                {
                    16_384
                }
                _ => {
                    return Ok(response(
                        StatusCode::BAD_REQUEST,
                        "Invalid Fero action".into(),
                    ));
                }
            };
            let Ok(body) = Limited::new(request.into_body(), limit).collect().await else {
                return Ok(response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Could not read request body".into(),
                ));
            };
            let body = body.to_bytes();
            if path == "/torrent" {
                WebAction::Torrent(body.to_vec())
            } else {
                let field = |key| {
                    url::form_urlencoded::parse(&body)
                        .find(|(name, _)| name == key)
                        .map(|(_, value)| value.into_owned())
                };
                match path.as_str() {
                    "/magnet" => WebAction::Magnet(field("magnet").unwrap_or_default()),
                    _ => {
                        let Some(id) = field("id").and_then(|id| id.parse().ok()) else {
                            return Ok(response(
                                StatusCode::BAD_REQUEST,
                                "Invalid transfer ID".into(),
                            ));
                        };
                        WebAction::Pause(id, path == "/pause")
                    }
                }
            }
        }
        Method::POST => {
            return Ok(response(
                StatusCode::FORBIDDEN,
                "Open Fero before sending a command".into(),
            ));
        }
        _ => return Ok(response(StatusCode::NOT_FOUND, "Page not found".into())),
    };
    if !authorized() {
        return Ok(response(
            StatusCode::UNAUTHORIZED,
            "Fero credentials changed".into(),
        ));
    }
    let listing = matches!(action, WebAction::List);
    let (reply, received) = oneshot::channel();
    let result = async {
        commands.send(Command::WebAction(action, reply)).ok()?;
        received.await.ok()
    }
    .await;
    Ok(match result {
        Some(Ok((rows, accent, reduce_motion))) if listing => {
            response(StatusCode::OK, page(rows, accent, reduce_motion))
        }
        Some(Ok(_)) => {
            let mut reply = response(StatusCode::SEE_OTHER, String::new());
            reply
                .headers_mut()
                .insert(header::LOCATION, "/".parse().unwrap());
            reply
        }
        Some(Err(error)) => response(
            StatusCode::BAD_REQUEST,
            format!(
                "<!doctype html><meta name=viewport content='width=device-width, initial-scale=1'><p>{}</p><a href='/'>Back to Fero</a>",
                escape(&error)
            ),
        ),
        None => response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Fero is unavailable".into(),
        ),
    })
}

fn response(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "same-origin")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        )
        .body(Full::new(Bytes::from(body)))
        .expect("Static HTTP headers")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(rows: Vec<Transfer>, accent: Accent, reduce_motion: bool) -> String {
    use crate::about;
    use std::fmt::Write;
    let palette = if accent == Accent::Harbor {
        "harbor"
    } else {
        "warm"
    };
    let mut page = include_str!("web_page.html")
        .replace("FERO_PALETTE", palette)
        .replace(
            "FERO_MOTION",
            if reduce_motion { "reduced" } else { "auto" },
        )
        .replace("<!-- FERO_BRAND -->", BRAND_SVG);
    if rows.is_empty() {
        page.push_str("<p>No transfers yet.</p>");
    }
    for row in rows {
        let progress = if row.progress.is_finite() {
            (row.progress * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let (action, label, pending) = if row.paused {
            ("resume", "Resume", "Resuming…")
        } else {
            ("pause", "Pause", "Pausing…")
        };
        let _ = write!(
            page,
            "<article><h2>{}</h2><progress aria-label='Transfer progress' max='100' value='{progress:.1}'></progress><div class='row'><p>{} · {progress:.1}% · ↓ {:.1} / ↑ {:.1} KiB/s</p><form action='/{action}' method='post'><input type='hidden' name='id' value='{}'><button data-pending='{pending}'>{label}</button></form></div></article>",
            escape(&row.name),
            escape(&row.state),
            f64::from(row.download_rate) / 1024.0,
            f64::from(row.upload_rate) / 1024.0,
            row.id,
        );
    }
    let _ = write!(
        page,
        "<footer><details><summary>About Fero</summary><p>Version {} · Local network controls for the Fero BitTorrent client.</p><p>{}</p><p>{}</p><p>{} <a href='{}' rel='noreferrer'>Solar Icons source</a> · <a href='{}' rel='noreferrer'>CC BY 4.0 license</a></p><details><summary>License notices</summary>",
        about::VERSION,
        escape(about::NAME_ORIGIN),
        escape(about::CORE_CREDIT),
        escape(about::SOLAR_CREDIT),
        about::SOLAR_SOURCE,
        about::SOLAR_LICENSE,
    );
    for &(name, text) in about::NOTICES {
        let _ = write!(
            page,
            "<details><summary>{}</summary><pre>{}</pre></details>",
            escape(name),
            escape(text),
        );
    }
    page.push_str("</details></details></footer></main></body></html>");
    page
}

#[cfg(test)]
#[test]
fn lan_page_includes_about_and_license_notices() {
    let html = page(Vec::new(), Accent::Harbor, false);
    assert!(html.contains("<summary>About Fero</summary>"));
    assert!(html.contains(&format!("Version {}", crate::about::VERSION)));
    assert!(html.contains("Solar Icons by 480 Design"));
    for &(name, text) in crate::about::NOTICES {
        assert!(html.contains(&format!(
            "<summary>{}</summary><pre>{}</pre>",
            escape(name),
            escape(text)
        )));
    }
}
