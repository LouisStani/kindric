//! The models Ollama has on this computer, for the form that adds an account whose model comes
//! from it (decision 418): one request to its local server, `GET /api/tags`. Nothing is loaded:
//! the list costs no memory.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where Ollama listens by default, the address kindric-accounts gives an `ollama` account.
const ADDRESS: ([u8; 4], u16) = ([127, 0, 0, 1], 11434);
const TIMEOUT: Duration = Duration::from_secs(2);
/// A list of models is a few KB; anything far larger is not Ollama.
const MAX_RESPONSE: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Model {
    /// The name an account uses, `qwen3.5:9b`.
    pub name: String,
    /// Its size on disk, in bytes.
    pub size: u64,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
    #[serde(default)]
    size: u64,
}

/// The models of the local Ollama, sorted by name. An error says how to get Ollama to answer.
pub fn models() -> io::Result<Vec<Model>> {
    let response = get_tags().map_err(|error| {
        io::Error::new(error.kind(), "Ollama does not answer on this computer: start it with `ollama serve`, or type the model's name")
    })?;
    parse_response(&response)
}

fn get_tags() -> io::Result<Vec<u8>> {
    let address = SocketAddr::from(ADDRESS);
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    // HTTP/1.0: the server closes the connection once the answer is sent, so it reads to the end.
    stream.write_all(b"GET /api/tags HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut response = Vec::new();
    stream.take(MAX_RESPONSE).read_to_end(&mut response)?;
    Ok(response)
}

/// The models of an HTTP answer to `GET /api/tags`.
fn parse_response(response: &[u8]) -> io::Result<Vec<Model>> {
    let text = String::from_utf8_lossy(response);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Err(io::Error::other("Ollama gave an answer that is not HTTP"));
    };
    let status = head.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("200") {
        return Err(io::Error::other(format!("Ollama answered {status:?} for its list of models")));
    }
    let tags: Tags = serde_json::from_str(body).map_err(|error| io::Error::other(format!("Ollama's list of models is not valid: {error}")))?;
    let mut models: Vec<Model> = tags.models.into_iter().map(|tag| Model { name: tag.name, size: tag.size }).collect();
    models.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_are_read_from_the_answer_and_sorted() {
        let response = b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"models\":[{\"name\":\"qwen3.5:9b\",\"size\":6600000000,\"details\":{}},{\"name\":\"gemma4:26b\",\"size\":18700000000}]}";
        let models = parse_response(response).unwrap();
        assert_eq!(
            models,
            [Model { name: "gemma4:26b".into(), size: 18_700_000_000 }, Model { name: "qwen3.5:9b".into(), size: 6_600_000_000 }]
        );
    }

    #[test]
    fn no_model_is_an_empty_list() {
        assert!(parse_response(b"HTTP/1.1 200 OK\r\n\r\n{\"models\":[]}").unwrap().is_empty());
    }

    #[test]
    fn an_error_status_or_another_answer_is_refused() {
        let status = parse_response(b"HTTP/1.1 404 Not Found\r\n\r\n404 page not found").unwrap_err().to_string();
        assert!(status.contains("404"), "{status}");
        assert!(parse_response(b"garbage").is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\n\r\n<html>").is_err());
    }
}
