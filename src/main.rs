#![deny(clippy::pedantic)]
#![allow(clippy::unused_async)]

mod errors;
mod highlight;
mod io;
mod params;

use crate::{
    errors::{InternalServerError, NotFound},
    highlight::highlight,
    io::{generate_id, get_paste, store_paste, PasteStore},
    params::{HostHeader, IsPlaintextRequest},
};

use actix_web::{
    http::header,
    web::{self, Bytes, Data, FormConfig, PayloadConfig},
    App, Error, HttpRequest, HttpResponse, HttpServer, Responder,
};
use askama::{Html as AskamaHtml, MarkupDisplay, Template};
use log::{error, info};
use once_cell::sync::Lazy;
use std::{
    borrow::Cow,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};
use syntect::html::{css_for_theme_with_class_style, ClassStyle};

#[derive(argh::FromArgs, Clone)]
/// a pastebin.
pub struct BinArgs {
    #[argh(
        positional,
        default = "SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 7278)"
    )]
    bind_addr: SocketAddr,
    /// maximum amount of pastes to store before rotating (default: 1000)
    #[argh(option, default = "1000")]
    buffer_size: usize,
    /// maximum paste size in bytes (default. 32kB)
    #[argh(option, default = "32 * 1024")]
    max_paste_size: usize,
    /// base path for the application (default: "-")
    #[argh(option, default = "\"-\".to_string()")]
    base_path: String,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "INFO");
    }
    pretty_env_logger::init();

    let args: BinArgs = argh::from_env();

    let store = Data::new(PasteStore::default());

    let server = HttpServer::new({
        let args = args.clone();

        move || {
            App::new()
                .app_data(store.clone())
                .app_data(PayloadConfig::default().limit(args.max_paste_size))
                .app_data(FormConfig::default().limit(args.max_paste_size))
                .app_data(Data::new(args.clone()))
                .wrap(actix_web::middleware::Compress::default())
                .service(
                    web::scope(&format!("/{}", args.base_path.trim_start_matches('/')))
                        .route("", web::get().to(index))
                        .route("/", web::get().to(index))
                        .route("", web::post().to(submit))
                        .route("/", web::post().to(submit))
                        .route("", web::put().to(submit_raw))
                        .route("/", web::put().to(submit_raw))
                        .route("", web::head().to(HttpResponse::MethodNotAllowed))
                        .route("/highlight.css", web::get().to(highlight_css))
                        .route("/{paste}", web::get().to(show_paste))
                        .route("/{paste}", web::head().to(HttpResponse::MethodNotAllowed))
                )
                .default_service(web::to(|req: HttpRequest| async move {
                    error!("Couldn't find resource {}", req.uri());
                    HttpResponse::from_error(NotFound)
                }))
        }
    });

    info!("Listening on http://{}", args.bind_addr);

    server.bind(args.bind_addr)?.run().await
}

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    base_path: String,
}

async fn index(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Get the base path from app data
    let app_data = req.app_data::<Data<BinArgs>>().cloned();
    let base_path = match app_data {
        Some(data) => {
            let path = data.base_path.clone();
            if path.starts_with('/') { path } else { format!("/{}", path) }
        },
        None => "/-".to_string(),
    };
    
    render_template(&req, &Index { base_path })
}

#[derive(serde::Deserialize)]
struct IndexForm {
    val: Bytes,
}

async fn submit(input: web::Form<IndexForm>, store: Data<PasteStore>, req: HttpRequest) -> impl Responder {
    let id = generate_id();
    
    // Get the base path from app data
    let app_data = req.app_data::<Data<BinArgs>>().cloned();
    let base_path = match app_data {
        Some(data) => {
            let path = data.base_path.clone();
            if path.starts_with('/') { path } else { format!("/{}", path) }
        },
        None => "/-".to_string(),
    };
    
    let uri = format!("{base_path}/{id}");
    store_paste(&store, id, input.into_inner().val);
    HttpResponse::Found()
        .append_header((header::LOCATION, uri))
        .finish()
}

async fn submit_raw(
    data: Bytes,
    host: HostHeader,
    store: Data<PasteStore>,
    req: HttpRequest,
) -> Result<String, Error> {
    let id = generate_id();
    
    // Get the base path from app data
    let app_data = req.app_data::<Data<BinArgs>>().cloned();
    let base_path = match app_data {
        Some(data) => {
            let path = data.base_path.clone();
            if path.starts_with('/') { path } else { format!("/{}", path) }
        },
        None => "/-".to_string(),
    };
    
    let uri = if let Some(Ok(host)) = host.0.as_ref().map(|v| std::str::from_utf8(v.as_bytes())) {
        format!("https://{host}{base_path}/{id}\n")
    } else {
        format!("{base_path}/{id}\n")
    };

    store_paste(&store, id, data);

    Ok(uri)
}

#[derive(Template)]
#[template(path = "paste.html")]
struct ShowPaste<'a> {
    content: MarkupDisplay<AskamaHtml, Cow<'a, String>>,
    base_path: String,
}

async fn show_paste(
    req: HttpRequest,
    key: actix_web::web::Path<String>,
    plaintext: IsPlaintextRequest,
    store: Data<PasteStore>,
) -> Result<HttpResponse, Error> {
    let mut splitter = key.splitn(2, '.');
    let key = splitter.next().unwrap();
    let ext = splitter.next();

    let entry = get_paste(&store, key).ok_or(NotFound)?;

    // Get the base path from app data
    let app_data = req.app_data::<Data<BinArgs>>().cloned();
    let base_path = match app_data {
        Some(data) => {
            let path = data.base_path.clone();
            if path.starts_with('/') { path } else { format!("/{}", path) }
        },
        None => "/-".to_string(),
    };

    if *plaintext {
        Ok(HttpResponse::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(entry))
    } else {
        let data = std::str::from_utf8(entry.as_ref())?;

        let code_highlighted = match ext {
            Some(extension) => match highlight(data, extension) {
                Some(html) => html,
                None => return Err(NotFound.into()),
            },
            None => htmlescape::encode_minimal(data),
        };

        // Add <code> tags to enable line numbering with CSS
        let html = format!(
            "<code>{}</code>",
            code_highlighted.replace('\n', "</code><code>")
        );

        let content = MarkupDisplay::new_safe(Cow::Borrowed(&html), AskamaHtml);

        render_template(&req, &ShowPaste { content, base_path })
    }
}

async fn highlight_css() -> HttpResponse {
    static CSS: Lazy<Bytes> = Lazy::new(|| {
        highlight::BAT_ASSETS.with(|s| {
            Bytes::from(
                css_for_theme_with_class_style(s.get_theme("OneHalfDark"), ClassStyle::Spaced)
                    .unwrap(),
            )
        })
    });

    HttpResponse::Ok()
        .content_type("text/css")
        .body(CSS.clone())
}

fn render_template<T: Template>(req: &HttpRequest, template: &T) -> Result<HttpResponse, Error> {
    match template.render() {
        Ok(html) => Ok(HttpResponse::Ok().content_type("text/html").body(html)),
        Err(e) => {
            error!("Error while rendering template for {}: {e}", req.uri());
            Err(InternalServerError(Box::new(e)).into())
        }
    }
}
