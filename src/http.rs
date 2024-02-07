use crate::io::HttpRequest;

pub fn get_http_auth(req: &HttpRequest) -> String {
    if let Some(auth) = req.headers.get("Authorization") {
        auth.clone()
    } else if let Some(auth) = req.headers.get("authorization") {
        auth.clone()
    } else {
        "demo".to_string()
    }
}
