use askama::Template;
use axum::{
    async_trait,
    body::{self, BoxBody, Full},
    extract::{rejection::FormRejection, FromRequest},
    headers::{HeaderName, Referer},
    http::{HeaderMap, HeaderValue, Request},
    response::{IntoResponse, Redirect, Response},
    routing::{get_service, MethodRouter},
    Form, TypedHeader,
};
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use tokio::signal;
use tower_http::services::ServeDir;
use tracing::error;
use validator::Validate;

use crate::{config::CONFIG, error::AppError, CURRENT_SHA256, GIT_COMMIT, VERSION};

use super::{fmt::md2html, Claim, SiteConfig};

pub(super) fn into_response<T: Template>(t: &T, ext: &str) -> Response<BoxBody> {
    match t.render() {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", ext)
            .body(body::boxed(Full::from(body)))
            .unwrap(),
        Err(err) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(body::boxed(Full::from(format!("{err}"))))
            .unwrap(),
    }
}

#[derive(Template)]
#[template(path = "error.html")]
struct PageError<'a> {
    page_data: PageData<'a>,
    status: String,
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::CaptchaError
            | AppError::NameExists
            | AppError::InnCreateLimit
            | AppError::UsernameInvalid
            | AppError::WrongPassword
            | AppError::ImageError(_)
            | AppError::Locked
            | AppError::Hidden
            | AppError::ReadOnly
            | AppError::ValidationError(_)
            | AppError::NoJoinedInn
            | AppError::AxumFormRejection(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::WriteInterval => StatusCode::TOO_MANY_REQUESTS,
            AppError::NonLogin => return Redirect::to("/signin").into_response(),
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Banned => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        error!("{}, {}", status, self);
        let site_config = SiteConfig::default();
        let page_data = PageData::new("Error", &site_config, None, false);
        let page_error = PageError {
            page_data,
            status: status.to_string(),
            error: self.to_string(),
        };

        into_response(&page_error, "html")
    }
}

pub(crate) async fn handler_404() -> impl IntoResponse {
    AppError::NotFound.into_response()
}

pub(crate) struct ValidatedForm<T>(pub(super) T);

#[async_trait]
impl<T, S, B> FromRequest<S, B> for ValidatedForm<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
    Form<T>: FromRequest<S, B, Rejection = FormRejection>,
    B: Send + 'static,
{
    type Rejection = AppError;

    async fn from_request(req: Request<B>, state: &S) -> Result<Self, Self::Rejection> {
        let Form(value) = Form::<T>::from_request(req, state).await?;
        value.validate()?;
        Ok(ValidatedForm(value))
    }
}

pub(crate) async fn home() -> impl IntoResponse {
    Redirect::to("/inn/0")
}

/// serve static directory
pub(crate) async fn serve_dir(path: &str) -> MethodRouter {
    let fallback = tower::service_fn(|_| async {
        Ok::<_, std::io::Error>(Redirect::to("/signin").into_response())
    });
    let srv = get_service(ServeDir::new(path).precompressed_gzip().fallback(fallback));
    srv.handle_error(|error: std::io::Error| async move {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unhandled internal error: {error}"),
        )
    })
}

static CSS: Lazy<String> = Lazy::new(|| {
    let mut css = include_str!("../../static/css/bulma.min.css").to_string();
    css.push('\n');
    css.push_str(include_str!("../../static/css/main.css"));
    css
});

pub(crate) async fn style() -> (HeaderMap, &'static str) {
    let mut headers = HeaderMap::new();

    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("text/css"),
    );
    headers.insert(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static("public, max-age=1209600, s-maxage=86400"),
    );

    (headers, &CSS)
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("signal received, starting graceful shutdown");
}

pub(super) struct PageData<'a> {
    pub(super) title: &'a str,
    pub(super) site_name: &'a str,
    pub(super) site_description: String,
    pub(super) claim: Option<Claim>,
    pub(super) has_unread: bool,
    pub(super) sha256: &'a str,
    pub(super) version: &'a str,
    pub(super) git_commit: &'a str,
    pub(super) footer_links: Vec<(&'a str, &'a str)>,
}

impl<'a> PageData<'a> {
    pub(super) fn new(
        title: &'a str,
        site_config: &'a SiteConfig,
        claim: Option<Claim>,
        has_unread: bool,
    ) -> Self {
        let mut footer_links = vec![];
        for (path, _, link) in &CONFIG.serve_dir {
            if !link.is_empty() {
                footer_links.push((path.as_str(), link.as_str()));
            }
        }
        let site_description = md2html(&site_config.description);
        Self {
            title,
            site_name: &site_config.site_name,
            site_description,
            claim,
            has_unread,
            sha256: &CURRENT_SHA256,
            version: VERSION,
            git_commit: GIT_COMMIT,
            footer_links,
        }
    }
}

pub(super) fn get_referer(header: Option<TypedHeader<Referer>>) -> Option<String> {
    if let Some(TypedHeader(r)) = header {
        let referer = format!("{r:?}");
        let trimed = referer
            .trim_start_matches("Referer(\"")
            .trim_end_matches("\")");
        Some(trimed.to_owned())
    } else {
        None
    }
}

pub struct ParamsPage {
    pub(super) anchor: usize,
    pub(super) n: usize,
    pub(super) is_desc: bool,
}
