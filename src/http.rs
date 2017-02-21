
extern crate httparse;
use std::collections::HashMap;
use std::io::Write;

pub struct Request {
    pub method: Option<String>,
    pub path: Option<String>,
    pub headers: HashMap<String, Vec<u8>>,
}

impl Request {
    pub fn new() -> Request {
        Request{
            method: None,
            path: None,
            headers: HashMap::new()
        }
    }

    pub fn parse(&mut self, buf: &[u8]) -> httparse::Result<usize> {
        let mut header_buf = [httparse::EMPTY_HEADER;32];
        let mut request = httparse::Request::new(&mut header_buf);
        let result = request.parse(buf);
        match result {
            Ok(httparse::Status::Complete(_)) => {
                for h in request.headers {
                    if h.name != "" {
                        self.headers.insert(h.name.to_string(), h.value.to_vec());
                    }
                }
                self.method = Some( request.method.unwrap().to_string() );
                self.path = Some( request.path.unwrap().to_string() );
                result
            }
            _ => result
        }
    }

    pub fn serialize(&self, buf: &mut Vec<u8>) -> Result<(), String> {
        let method = match self.method {
            Some(value) => value,
            _ => {return Err("Could not serialize HTTP response: no \"method\" field".to_string()); }
        };

        let path = match self.path {
            Some(value) => value,
            _ => {return Err("Could not serialize HTTP response: no \"path\" field".to_string()); }
        };

        if let Err(_) = write!(&mut buf, "{} {} HTTP/1.1\r\n", method, path) {
            return Err("Could not serialize HTTP response.".to_string());
        }

        for (name, value) in self.headers {
            buf.extend(name.into_bytes());
            buf.extend(b": ");
            buf.extend(value);
            buf.extend(b"\r\n");
        }
        buf.extend(b"\r\n");
        Ok(())
    }
}

pub struct Response {
    pub code: Option<String>,
    pub reason: Option<String>,
    pub headers: HashMap<String, Vec<u8>>
}

impl Response {
    pub fn new() -> Response {
        Response{
            code: None,
            reason: None,
            headers: HashMap::new()
        }
    }

    pub fn serialize(&self, buf: &mut Vec<u8>) -> Result<(), String> {
        let code = match self.code {
            Some(value) => value,
            _ => {return Err("Could not serialize response: No \"code\" field.".to_string()); }
        };

        let reason = match self.reason {
            Some(value) => value,
            _ => {return Err("Could not serialize response: No \"reason\" field.".to_string()); }
        };

        if let Err(_) = write!(&mut buf, "HTTP/1.1 {} {}\r\n", code, reason) {
            return Err("Could not serialize response.".to_string());
        }

        for (name, value) in self.headers {
            buf.extend(name.into_bytes());
            buf.extend(b": ");
            buf.extend(value);
            buf.extend(b"\r\n");
        }
        buf.extend(b"\r\n");
        Ok(())
    }
}
