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

use futures::{Sink, Stream};
use openssl::crypto::hash::{self, hash};
use serialize::base64::{FromBase64, STANDARD};
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

/*
trait Serializable {
    fn serialize(&self, buf: &mut Vec<u8>) -> Option<usize>;
}

impl<'h, 'b> Serializable for httparse::Response<'h, 'b> {
    fn serialize(&self, buf: &mut Vec<u8>) -> Option<usize> {
        match (self.version, self.code, self.reason) {
            (Some(version), Some(code), Some(reason)) => {
                let mut _buf = Vec::new();
                write!(&mut _buf, 
                    "HTTP/{} {} {}\r\n", 
                    self.version.unwrap(), 
                    self.code.unwrap(), 
                    self.reason.unwrap());
                let len = _buf.len();
                buf.append(_buf);
                return Some(len);
            }
            _ => {
                return None;
            }
        }
    }
}
*/

fn serialize_httpresponse(response: &httparse::Response, 
                          buf: &mut Vec<u8>) -> Option<usize>
{
    match (response.version, response.code, response.reason) {
        (Some(version), Some(code), Some(reason)) => {
            let mut _buf = Vec::new();
            write!(&mut _buf, 
                "HTTP/{} {} {}\r\n", 
                response.version.unwrap(), 
                response.code.unwrap(), 
                response.reason.unwrap());
            let len = _buf.len();
            buf.append(&mut _buf);
            return Some(len);
        }
        _ => {
            return None;
        }
    }
}

/*
enum WsRequest {
    Http(httparse::Request<'h, 'b>),
    WsFrame(Frame),
}

enum WsResponse<'h, 'b> {
    Http(httparse::Response<'h, 'b>),
    WsFrame(Frame),
}
*/

pub struct HttpCodec;

impl Codec for HttpCodec {
    type In = Box<HashMap<String, Vec<u8>>>; // Hash of headers/values
    type Out = String;

    fn decode(&mut self, buf: &mut EasyBuf) -> io::Result<Option<Self::In>> {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut r = httparse::Request::new(&mut headers);
        let status = try!(r.parse(buf.as_slice()).map_err(|e| {
                    let msg = format!("failed to parse http request: {:?}", e);
                    io::Error::new(io::ErrorKind::Other, msg)
                    }));

        let amt = match status {
            httparse::Status::Complete(amt) => amt,
            httparse::Status::Partial => return Ok(None),
        };

        let mut hashmap: HashMap<String, Vec<u8>> = HashMap::new();
        for header in r.headers.iter() {
            let mut name = String::new();
            name.push_str(header.name);
            hashmap.insert(
                name, 
                header.value.to_vec());
        }
        Ok(Some(Box::new(hashmap)))
    }

    fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.extend(msg.as_bytes());
        Ok(())
    }
}

pub enum WsFrame {
    HttpFrame(Vec<u8>), // Contains raw HTTP data
    WsFrame(Vec<u8>) // contains only payload portion of WS frame
}

pub struct WsCodec;

impl WsCodec {
    fn new() -> WsCodec {
        WsCodec{}
    }
}

impl Codec for WsCodec {
    type In = WsFrame;
    type Out = WsFrame;

    fn decode(&mut self, buf: &mut EasyBuf) -> io::Result<Option<Self::In>> {
        info!("Decode {} bytes.", buf.len());
        if buf.as_slice().starts_with(b"GET") {
            /* Parse HTTP Request */
            let data = buf.as_slice().to_vec().clone();
            return Ok(Some(WsFrame::HttpFrame(data)))
        } else {
            /* Parse WS Frame */
            let len = buf.len();
            let mut buf = buf.drain_to(len);
            let mut mutbuf = buf.get_mut();
            let result = ss::frame::WebSocketFrameBuilder::from_bytes(mutbuf.deref_mut());
            if let Some(boxed_frame) = result {
                return Ok(Some(WsFrame::WsFrame(boxed_frame.payload())));
            } else {
                return Err(io::Error::new(io::ErrorKind::Other, "Could not parse WS data frame"));
            }
        }
    }

	fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>)
			 -> io::Result<()>
	{
        match msg {
            WsFrame::HttpFrame(payload) => {
                buf.extend(payload);
            }
            WsFrame::WsFrame(payload) => {
                let frame = ss::frame::WebSocketFrame::new(
                    payload.as_slice(),
                    ss::frame::FrameType::Data,
                    ss::frame::OpType::Binary
                    );
                buf.extend(frame.to_bytes());
            }
        }
        Ok(())
	}
}

pub struct WsProto;

use tokio_core::io::{Io, Framed};

    // When created by TcpServer, "T" is "TcpStream"
impl<T: Io + 'static> ServerProto<T> for WsProto {
    /// For this protocol style, `Request` matches the codec `In` type
    type Request = WsFrame;

    /// For this protocol style, `Response` matches the coded `Out` type
    type Response = WsFrame;

    /// A bit of boilerplate to hook in the codec:
    type Transport = Framed<T, WsCodec>;
    type BindTransport = Box<Future<Item = Self::Transport, 
                                    Error = io::Error>>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        let transport = io.framed(WsCodec::new()); 

        transport.into_future()
            .map_err(|(e, _)| e)
            .and_then(|(frame, transport)| {
                /* This should be an HTTP GET request frame */
                match frame {
                    Some(WsFrame::HttpFrame(payload)) => {
                        /* Parse the HTTP request */
                        let mut headers = [httparse::EMPTY_HEADER; 32];
                        let mut req = httparse::Request::new(&mut headers);
                        if let Ok(httparse::Status::Complete(parsed_size)) = req.parse(payload.as_slice()) {
                            info!("Request parsed correctly. size: {}", parsed_size);
                            /* Send the reply */

                            let maybe_key = req.headers.iter().find(|h| { 
                                info!("Searching for header... {}", h.name);
                                h.name=="Sec-WebSocket-Key" 
                            });
                            if maybe_key.is_none() {
                                info!("Could not find Sec-WebSocket-Key header");
                                let err = io::Error::new(io::ErrorKind::Other, 
                                    "invalid handshake: No Sec-WebSocket-Key header found");
                                return Box::new(future::err(err)) as Self::BindTransport;
                            }

                            let key = maybe_key.unwrap();

                            /* Formulate the Server's response:
                               https://tools.ietf.org/html/rfc6455#section-1.3 */

                            /* First, formulate the "Sec-WebSocket-Accept" header field */
                            let mut concat_key = String::new();
                            concat_key.push_str(str::from_utf8(key.value).unwrap());
                            concat_key.push_str(MAGIC_GUID);
                            let output = hash(hash::Type::SHA1, concat_key.as_bytes());
                            /* Form the HTTP response */
                            let mut headers : [httparse::Header; 3] = [ 
                                httparse::Header{name: "Upgrade", value: b"websocket"},
                                httparse::Header{name: "Connection", value: b"Upgrade"},
                                httparse::Header{name: "Sec-WebSocket-Accept", value : output.as_slice() },
                                ];
                            let mut response = httparse::Response::new(&mut headers);
                            response.version = Some(1u8);
                            response.code = Some(101);
                            response.reason = Some("Switching Protocols");
                            let mut payload = Vec::new();
                            if let Some(msg_len) = serialize_httpresponse(&response, &mut payload) {
                                info!("Sending response...");
                                //return tokio_core::io::write_all(socket, payload).boxed();
                                return Box::new(transport.send(WsFrame::HttpFrame(payload))) as Self::BindTransport;
                            } else {
                                let err = io::Error::new(io::ErrorKind::Other, 
                                                         "Could not serialize response");
                                return Box::new(future::err(err)) as Self::BindTransport;
                            }
                        }
                    }
                    _ => {
                        let err = io::Error::new(io::ErrorKind::Other, "Invalid WebSocket handshake");
                        return Box::new(future::err(err)) as Self::BindTransport;
                    }
                }
            })
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

fn serve<S>(s: S)-> io::Result<()>
    where S: NewService<Request = Vec<u8>,
                        Response = Vec<u8>,
                        Error = io::Error> + 'static
{
    type WriteResult = Box<Future<Item=(tokio_core::net::TcpStream, Vec<u8>), Error=io::Error>>;
    let mut core = reactor::Core::new()?;
    let handle = core.handle();

    let address = "0.0.0.0:42001".parse().unwrap();
    let listener = tokio_core::net::TcpListener::bind(&address, &handle).unwrap();

    let connections = listener.incoming();
    let server = connections.for_each(move |(socket, _peer_addr)| {
        info!("Connection received.");
        /* Receive the HTTP handshake */
        let mut service = s.new_service()?;

        let transport = socket.framed(HttpCodec);

        let handshake = transport.into_future().map_err(|(e, _)| e).and_then(|(line, transport)| {
            type HandshakeResult = Box<Future<Item=(), Error=io::Error>>;
        });

        let handshake = transport.and_then(|http_map| {
            type HandshakeResult = Box<Future<Item=(), Error=io::Error>>;
            /*
            for (key, value) in http_map.iter() {
                println!("{}: {}", key, str::from_utf8(value.as_slice()).unwrap());
            }
            */
            /* Get the "Sec-WebSocket-Key" header */
            if !http_map.contains_key("Sec-WebSocket-Key") {
                let err = io::Error::new(io::ErrorKind::Other, 
                    "invalid handshake: Sec-WebSocket-Key header not found.");
                return future::err(err).boxed() as HandshakeResult
            }

            let key = http_map.get("Sec-WebSocket-Key").unwrap();

            /* Formulate the Server's response:
               https://tools.ietf.org/html/rfc6455#section-1.3 */

            /* First, formulate the "Sec-WebSocket-Accept" header field */
            let mut concat_key = String::new();
            concat_key.push_str(str::from_utf8(key).unwrap());
            concat_key.push_str(MAGIC_GUID);
            let output = hash(hash::Type::SHA1, concat_key.as_bytes());
            /* Form the HTTP response */
            let mut headers : [httparse::Header; 3] = [ 
                httparse::Header{name: "Upgrade", value: b"websocket"},
                httparse::Header{name: "Connection", value: b"Upgrade"},
                httparse::Header{name: "Sec-WebSocket-Accept", value : output.as_slice() },
                ];
            let mut response = httparse::Response::new(&mut headers);
            response.version = Some(1u8);
            response.code = Some(101);
            response.reason = Some("Switching Protocols");
            let mut payload = Vec::new();
            if let Some(msg_len) = serialize_httpresponse(&response, &mut payload) {
                info!("Sending response...");
                /* TODO */
            } else {
                let err = io::Error::new(io::ErrorKind::Other, 
                                         "Could not serialize response");
                return Box::new(future::err(err));
            }

            Box::new(future::ok(())) as HandshakeResult
        }).into_future().map( |_| () ).map_err(|_| ());

        handle.spawn(handshake);

        Ok(())

    });

    core.run(server)
}
                        
                        

fn main() {
    env_logger::init().unwrap();
    println!("Server starting...");
    info!("Moop!");
    // Specify the localhost address
    //let addr = "0.0.0.0:12345".parse().unwrap();

    if let Err(e) = serve(|| Ok(Echo)) {
        println!("Server failed with {}", e);
    }

    // The builder requires a protocol and an address
    // let server = TcpServer::new(WsProto, addr);

    // We provide a way to *instantiate* the service for each new
    // connection; here, we just immediately return a new instance.
    // server.serve(|| Ok(Echo));
}

