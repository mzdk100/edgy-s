use {
    hyper::Uri,
    percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode},
    std::{
        any::type_name,
        collections::HashMap,
        io::{Error as IoError, Result as IoResult},
    },
};

/// Characters that need to be percent-encoded in URL query strings
const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}')
    .add(b'~');

/// Generates a URL path from a type name.
///
/// Converts the type name to a URL path by:
/// 1. Removing generic parameters (e.g., `<T>` becomes empty)
/// 2. Replacing `::` with `/`
/// 3. Replacing `_` with `/`
/// 4. Prepending `/`
///
/// # Example
/// ```
/// use edgy_s::utils::get_path;
///
/// struct MyHandler;
/// assert_eq!(get_path::<MyHandler>(), "/MyHandler");
///
/// mod api { pub struct User; }
/// assert_eq!(get_path::<api::User>(), "/api/User");
/// ```
pub fn get_path<T>() -> String {
    let name = type_name::<T>();
    let name = if let Some(i) = name.find("<") {
        &name[..i]
    } else {
        name
    };
    format!(
        "/{}",
        name.split("::").last().unwrap_or("").replace("_", "/")
    )
}

/// URL Decodes a percent-encoded string using `percent-encoding` crate.
///
/// Converts `%XX` sequences to their corresponding bytes and `+` to space.
/// Properly handles UTF-8 encoded characters.
///
/// # Arguments
/// * `encoded` - The percent-encoded string
///
/// # Returns
/// The decoded string or an error if the encoding is invalid
///
/// # Examples
/// ```
/// # use percent_encoding::percent_decode_str;
/// # fn url_decode(encoded: &str) -> Result<String, String> {
/// #     percent_decode_str(&encoded.replace('+', "%20"))
/// #         .decode_utf8()
/// #         .map(|cow| cow.into_owned())
/// #         .map_err(|e| e.to_string())
/// # }
/// assert_eq!(url_decode("hello%20world").unwrap(), "hello world");
/// assert_eq!(url_decode("hello+world").unwrap(), "hello world");
/// assert_eq!(url_decode("%E4%B8%AD%E6%96%87").unwrap(), "中文");
/// ```
pub fn url_decode(encoded: &str) -> Result<String, String> {
    // Replace '+' with '%20' for application/x-www-form-urlencoded compatibility
    percent_decode_str(&encoded.replace('+', "%20"))
        .decode_utf8()
        .map(|cow| cow.into_owned())
        .map_err(|e| e.to_string())
}

/// Build a full URI by combining base_url and path
///
/// # Arguments
/// * `base_url` - The base URL to combine with the path
/// * `path` - The path to append to the base URL
/// * `force_scheme` - If provided, overrides the scheme from base_url.
///                    Automatically adds 's' suffix for TLS (e.g., "http" → "https" if base_url uses TLS)
pub fn build_uri<P>(base_url: &Uri, path: P, force_scheme: Option<&str>) -> IoResult<Uri>
where
    P: AsRef<str>,
{
    let is_tls = matches!(base_url.scheme_str(), Some("https" | "wss"));
    let scheme = if let Some(forced) = force_scheme {
        if is_tls && !forced.ends_with('s') {
            format!("{}s", forced)
        } else {
            forced.to_owned()
        }
    } else {
        base_url.scheme_str().unwrap_or_default().to_owned()
    };

    let default_port = if is_tls { 443 } else { 80 };
    let base = format!(
        "{}://{}:{}{}",
        scheme,
        base_url.host().unwrap_or_default(),
        base_url.port_u16().unwrap_or(default_port),
        base_url.path()
    );

    let url = if base.ends_with('/') {
        format!("{}{}", base.trim_end_matches('/'), path.as_ref())
    } else {
        format!("{}{}", base, path.as_ref())
    };

    let full_url = format!(
        "{}{}",
        url,
        base_url
            .query()
            .map(|q| format!("?{}", q))
            .unwrap_or_default()
    );

    full_url.parse().map_err(IoError::other)
}

/// Append query parameters to a URI
///
/// # Arguments
/// * `uri` - The base URI
/// * `params` - The query parameters to append
///
/// # Returns
/// A new URI with the query parameters appended
pub fn append_query_params(uri: &Uri, params: &HashMap<String, String>) -> Uri {
    if params.is_empty() {
        return uri.clone();
    }

    let query_string: String = params
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, QUERY_ENCODE_SET),
                utf8_percent_encode(v, QUERY_ENCODE_SET)
            )
        })
        .collect::<Vec<_>>()
        .join("&");

    let existing_query = uri.query().unwrap_or("");
    let new_query = if existing_query.is_empty() {
        query_string
    } else {
        format!("{}&{}", existing_query, query_string)
    };

    let new_uri = format!(
        "{}://{}{}?{}",
        uri.scheme_str().unwrap_or("http"),
        uri.authority().map(|a| a.as_str()).unwrap_or("localhost"),
        uri.path(),
        new_query
    );

    new_uri.parse().unwrap_or_else(|_| uri.clone())
}
