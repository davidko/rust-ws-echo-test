extern crate env_logger;
extern crate futures;
extern crate httparse;
#[macro_use]
extern crate log;
extern crate openssl;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate rustc_serialize as serialize;
extern crate simple_stream as ss;

mod http;

use futures::{Sink, Stream};
use openssl::crypto::hash::{self, hash};
use serialize::base64::{FromBase64, ToBase64, STANDARD};
use ss::frame::Frame;
use ss::frame::FrameBuilder;
use std::collections::HashMap;
//use std::fmt::Write;
use std::io::Write;
use std::io;
use std::ops::DerefMut;
use std::str;
use tokio_core::io::{Codec, EasyBuf};

use tokio_proto::pipeline::ServerProto;

static MAGIC_GUID: &'static str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub struct HttpCodec;

impl Codec for HttpCodec {
    type In = http::Request;
    type Out = http::Response;

    fn decode(&mut self, buf: &mut EasyBuf) -> io::Result<Option<Self::In>> {
        info!("Trying to decode HTTP frame...");
        let mut request = http::Request::new();
        match request.parse(buf.as_slice()) {
            Ok(httparse::Status::Complete(_)) => Ok(Some(request)),
            Ok(httparse::Status::Partial) => Ok(None),
            _ => Err(io::Error::new(io::ErrorKind::Other, "Could not parse HTTP request"))
        }
    }

    fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>) -> io::Result<()> {
        msg.serialize(buf);
        let buf = buf.clone();
        info!("Sending message: {}", String::from_utf8(buf).unwrap());
        Ok(())
    }
}

pub struct WsCodec;

impl WsCodec {
    fn new() -> WsCodec {
        WsCodec{}
    }
}

impl Codec for WsCodec {
    type In = Vec<u8>;
    type Out = Vec<u8>;

    fn decode(&mut self, buf: &mut EasyBuf) -> io::Result<Option<Self::In>> {
        info!("Decode {} bytes.", buf.len());
        /* Parse WS Frame */
        let len = buf.len();
        let mut buf = buf.drain_to(len);
        let mut mutbuf = buf.get_mut();
        let result = ss::frame::WebSocketFrameBuilder::from_bytes(mutbuf.deref_mut());
        if let Some(boxed_frame) = result {
            return Ok(Some(boxed_frame.payload()));
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "Could not parse WS data frame"));
        }
    }

	fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>)
			 -> io::Result<()>
	{
        let frame = ss::frame::WebSocketFrame::new(
            msg.as_slice(),
            ss::frame::FrameType::Data,
            ss::frame::OpType::Binary
            );
        buf.extend(frame.to_bytes());
        Ok(())
	}
}

pub struct WsProto;

use tokio_core::io::{Io, Framed};

    // When created by TcpServer, "T" is "TcpStream"
impl<T: Io + Send + 'static> ServerProto<T> for WsProto {
    /// For this protocol style, `Request` matches the codec `In` type
    type Request = Vec<u8>;

    /// For this protocol style, `Response` matches the coded `Out` type
    type Response = Vec<u8>;

    /// A bit of boilerplate to hook in the codec:
    type Transport = Framed<T, WsCodec>;
    type BindTransport = Box<Future<Item = Self::Transport, 
                                    Error = io::Error>>;


    fn bind_transport(&self, io: T) -> Self::BindTransport {
        let transport = io.framed(HttpCodec); 

        let handshake = transport.into_future()
            .map_err(|(e, _)| e)
            .and_then(|(frame, transport)| {
                /* This should be an HTTP GET request frame */
                if frame.is_none() {
                    let err = io::Error::new(io::ErrorKind::Other, 
                        "Invalid WebSocket handshake: Expected HTTP frame.");
                    return Box::new(future::err(err)) as Box<Future<Item=_, Error=io::Error>>;
                }
                /* Get the Sec-WebSocket-Key header */
                let frame = frame.unwrap();
                let key = match frame.headers.iter().find(|h| { 
                    info!("Searching for header... {}", h.0);
                    h.0=="Sec-WebSocket-Key" 
                }) {
                    Some(tuple) => tuple.1,
                    None => {
                        info!("Could not find Sec-WebSocket-Key header!");
                        let err = io::Error::new(io::ErrorKind::Other, 
                            "Invalid WebSocket handshake: No Sec-WebSocket-Key header found.");
                        return Box::new(future::err(err));
                    }
                };

                /* Formulate the Server's response:
                   https://tools.ietf.org/html/rfc6455#section-1.3 */

                /* First, formulate the "Sec-WebSocket-Accept" header field */
                let mut concat_key = String::new();
                concat_key.push_str(str::from_utf8(key).unwrap());
                concat_key.push_str(MAGIC_GUID);
                let output = hash(hash::Type::SHA1, concat_key.as_bytes());
                /* Form the HTTP response */
                let mut response = http::Response::new();
                response.code = Some("101".to_string());
                response.reason = Some("Switching Protocols".to_string());
                response.headers.insert("Upgrade".to_string(), b"websocket".to_vec());
                response.headers.insert("Connection".to_string(), b"Upgrade".to_vec());
                response.headers.insert("Sec-WebSocket-Accept".to_string(), output.to_base64(STANDARD).as_bytes().to_vec());
                return Box::new(transport.send(response));
            }).and_then(|transport| {
                let socket = transport.into_inner();
                let ws_transport = socket.framed(WsCodec);
                return Box::new(future::ok(ws_transport)) as Self::BindTransport;
            });

        Box::new(handshake)
        // transport : Framed<Self: TcpStream, C: WsCodec>
        //let err = io::Error::new(io::ErrorKind::Other, "blah");
        //Box::new(future::err(err)) as Self::BindTransport
    }
}

use tokio_service::Service;

pub struct Echo;

use futures::{future, Future, BoxFuture};

impl Service for Echo {
    // These types must match the corresponding protocol types:
    type Request = Vec<u8>;
    type Response = Vec<u8>;

    // For non-streaming protocols, service errors are always io::Error
    type Error = io::Error;

    // The future for computing the response; box it for simplicity.
    type Future = BoxFuture<Self::Response, Self::Error>;

    // Produce a future for computing a response from a request.
    fn call(&self, req: Self::Request) -> Self::Future {
        // In this case, the response is immediate.
        // We get a frame in. Get the data

        /* 
        info!("Service call.");
        let payload = req.payload();
        let mut s = String::new();
        for byte in &payload {
            write!(s, "{:X} ", byte);
        }
        info!("{}", s);
        future::ok(payload).boxed()
        */

        //future::ok(req).boxed()

        future::ok(Vec::new()).boxed()
    }
}

use tokio_proto::TcpServer;

use tokio_core::reactor;
use tokio_core::net;
use tokio_service::{NewService};


fn main() {
    env_logger::init().unwrap();
    println!("Server starting...");
    info!("Moop!");
    // Specify the localhost address
    let addr = "0.0.0.0:12345".parse().unwrap();

    // The builder requires a protocol and an address
     let server = TcpServer::new(WsProto, addr);

    // We provide a way to *instantiate* the service for each new
    // connection; here, we just immediately return a new instance.
     server.serve(|| Ok(Echo));
}

