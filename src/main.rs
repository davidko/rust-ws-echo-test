extern crate futures;
extern crate httparse;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate simple_stream as ss;

use futures::Stream;
use ss::frame::Frame;
use ss::frame::FrameBuilder;
//use std::fmt::Write;
use std::io::Write;
use std::io;
use std::ops::DerefMut;
use std::str;
use tokio_core::io::{Codec, EasyBuf};

use tokio_proto::pipeline::ServerProto;

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

pub struct WsCodec
{
    // "initialized" is false pre-handshake, true post-handshake
    initialized: bool
}

impl Codec for WsCodec {
    type In = Vec<u8>;
    type Out = Vec<u8>;

    fn decode(&mut self, buf: &mut EasyBuf) -> io::Result<Option<Self::In>> {
        println!("Decode {} bytes.", buf.len());
        if self.initialized {
            println!("{}", String::from_utf8(buf.as_slice().to_vec()).unwrap());
            let len = buf.len();
            let mut buf = buf.drain_to(len);
            let mut mutbuf = buf.get_mut();
            let result = ss::frame::WebSocketFrameBuilder::from_bytes(mutbuf.deref_mut());
            if let Some(boxed_frame) = result {
                Ok(Some(boxed_frame.payload()))
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "Could not parse WS data frame"))
            }
        } else {
            Ok(None)
        }

/*
         if let Some(i) = buf.as_slice().iter().position(|&b| b == b'\n') {
             // remove the serialized frame from the buffer.
             let line = buf.drain_to(i);

             // Also remove the '\n'
             buf.drain_to(1);

             // Turn this data into a UTF string and return it in a Frame.
             match str::from_utf8(line.as_slice()) {
                 Ok(s) => Ok(Some(s.to_string())),
                     Err(_) => Err(io::Error::new(io::ErrorKind::Other,
                                 "invalid UTF-8")),
             }
         } else {
             Ok(None)
         }
*/
    }

	fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>)
			 -> io::Result<()>
	{
        println!("Encode.");
        let frame = ss::frame::WebSocketFrame::new(
            msg.as_slice(),
            ss::frame::FrameType::Data,
            ss::frame::OpType::Binary
            );
        buf.extend(frame.to_bytes());
        Ok(())
        /*
		buf.extend(msg.as_bytes());
		buf.push(b'\n');
		Ok(())
        */
	}
}

pub struct WsProto;

use tokio_core::io::{Io, Framed};

    // When created by TcpServer, "T" is "TcpStream"
impl<T: Io + 'static> ServerProto<T> for WsProto {
    /// For this protocol style, `Request` matches the codec `In` type
    type Request = Vec<u8>;

    /// For this protocol style, `Response` matches the coded `Out` type
    type Response = Vec<u8>;

    /// A bit of boilerplate to hook in the codec:
    type Transport = Framed<T, WsCodec>;
    type BindTransport = Box<Future<Item = Self::Transport, 
                                    Error = io::Error>>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        let transport = io.framed(WsCodec{initialized: false}); 
        // transport : Framed<Self: TcpStream, C: WsCodec>

        // handshake : StreamFuture< Framed<TcpStream, WsCodec> > where Item: Framed::Item = WsCodec::In
        let handshake = transport.into_future()
            // We don't care about the second item of the error tuple, which is
            // the transport, if it errors out.
            .map_err(|(e, _)| e)
            .and_then(|(line, transport)| {
                match line {
                    Some(ref msg) => {
                        /* Parse the incoming http request */
                        let mut headers = [httparse::EMPTY_HEADER; 16];
                        let mut req = httparse::Request::new(&mut headers);
                        let res = req.parse(msg.as_slice());

                        /* FIXME */
                        let err = io::Error::new(io::ErrorKind::Other, "invalid handshake");
                        Box::new(future::err(err)) as Self::BindTransport
                    }
                    _ => {
                        let err = io::Error::new(io::ErrorKind::Other, "invalid handshake");
                        Box::new(future::err(err)) as Self::BindTransport
                    }
                }
            });
        //Ok(io.framed(WsCodec))

        Box::new(handshake)
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
        println!("Service call.");
        let payload = req.payload();
        let mut s = String::new();
        for byte in &payload {
            write!(s, "{:X} ", byte);
        }
        println!("{}", s);
        future::ok(payload).boxed()
        */

        //future::ok(req).boxed()

        future::ok(Vec::new()).boxed()
    }
}

use tokio_proto::TcpServer;

fn main() {
    println!("Server starting...");
    // Specify the localhost address
    let addr = "0.0.0.0:12345".parse().unwrap();

    // The builder requires a protocol and an address
    let server = TcpServer::new(WsProto, addr);

    // We provide a way to *instantiate* the service for each new
    // connection; here, we just immediately return a new instance.
    // server.serve(|| Ok(Echo));
}

