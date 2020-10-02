use pyo3::prelude::*;
use pyo3::PyIterProtocol;
use pyo3::class::pyasync::PyAsyncProtocol;
use pyo3::class::iter::IterNextOutput;

use std::net::{TcpListener, TcpStream};
use std::io;
use std::io::prelude::*;
use std::collections::HashMap;
use bstr::ByteSlice;


///
/// just aquires the event loop by import asyncio 
/// and then calling get_event_loop()
/// this is equivelent to loop = asyncio.get_event_loop()
/// in python returning a result should asyncio not exist 
/// (just a rust thing)
///
fn get_loop(py: Python) -> PyResult<&PyAny> {
    let asyncio = py.import("asyncio")?;
    Ok(asyncio.call0("get_event_loop")?)
}

///
/// AsynServer represents the actual Rust TCP listener 
/// it initially binds to the address on creation with new(),
/// accept_client() can be called to get the next tcp stream,
/// because this is for asyncio we want this to be non-blocking so
/// we set none-blocking on the socket. accept_client will return 
/// either None or a TcpStream.
/// 
/// ```
/// let server = AsyncServer::new("127.0.0.1:8080");
/// 
/// let next_client = server.accept_client();
/// println!("{:?}", next_client);
/// ```
///  
struct AsyncServer {
    listener: TcpListener,
}

impl AsyncServer {
    fn new(addr: String) -> Self {
        let listener = TcpListener::bind(addr).unwrap();
        listener.set_nonblocking(true).expect("Cannot set non-blocking");

        Self { listener }
    }

    fn accept_client(&mut self) -> Option<TcpStream> {
        return match self.listener.incoming().next() {
            Some(s) => {
                match s {
                    Ok(res) => Some(res),
                    Err(ref er) if er.kind() == io::ErrorKind::WouldBlock => None,
                    Err(er) => {
                        eprintln!("{}", er);
                        None
                    },
                }
            }
            _ => {
                None
            }
        };
    }
}


///
/// The AsyncServerRunner struct houses the TCP listener and sparks the async tasks,
/// it has a integral clock delay set to n to save cpu todo: find the right match.
///
#[pyclass]
struct AsyncServerRunner {
    // External inputs
    callback: PyObject,

    // Internal systems
    server: AsyncServer,        // The non-blocking TCP listener Struct
    server_state: u8,           // A int representing the asyncio state, either 0, 1, 2 or Error
    server_exit: bool,          // A bool to signal if the server should shutdown and return
    loop_: PyObject,            // The asyncio event loop
    fut: Option<Py<PyAny>>,     // The temporary future to house the sleep future to save CPU
    internal_clock_delay: f32,  // the delay between loop iterations.

}


#[pymethods]
impl AsyncServerRunner {

    ///
    /// PythonMethod: AsyncServerRunner::new() -> Self
    /// 
    ///     new() creates the AsyncServer instance and aquires the asyncio
    ///     event loop, default state is set to `0`, server exit `false`,
    ///     clock delay `0.05`.
    /// 
    ///     Requires:
    ///         - binding_addr: String
    ///         - callback:     PyObject
    ///
    #[new]
    fn new(binding_addr: String, callback: PyObject) -> Self {
        println!("Connecting to {}", &binding_addr);

        let server = AsyncServer::new(binding_addr);
        let loop_ = {
            let gil = Python::acquire_gil();
            let py = gil.python();
            get_loop(py).unwrap().into_py(py)
        };

        AsyncServerRunner {
            server,
            server_state: 0,
            server_exit: false,
            loop_,
            fut: None,
            internal_clock_delay: 0.01,
            callback,
        }
    }
}


///  
/// This implementation houses the intenal functions for creating a non-blocking
/// delay on the event loop to save cpu.
///   
impl AsyncServerRunner {

    /// 
    /// Internal Method: AsyncServerRunner._sleep() -> PyResult<()>
    ///     
    ///     _sleep recreated what asyncio.sleep() does, internally
    ///     it calls loop.create_future() on the running event loop, aquires the 
    ///     asyncio.futures module, and then calles loop.call_later() using
    ///     `AsyncServerRunner.internal_clock_delay` as the delay to then invoke
    ///     future's private method `_set_result_unless_cancelled`. After the future
    ///     has been set we just set the future to the iterator to yeild from.
    ///     
    ///     Note:
    ///         I used `_set_result_unless_cancelled` because I was getting
    ///         a error or it just not waiting at all with set_result or using
    ///         a normal callback, this system is just a plain copy of asyncio.sleep.
    ///         
    ///     Requires:
    ///         - py: Python
    /// 
    fn _sleep(&mut self, py: Python) -> PyResult<()> {
        self.fut = Option::from(self.loop_.call_method0(py, "create_future")?);

        let futures = py.import("asyncio")?.get("futures")?;
        let _ = self.loop_.call_method1(
            py,
            "call_later",
            (
                self.internal_clock_delay,
                futures.getattr("_set_result_unless_cancelled")?,
                self.fut.as_ref(),
                "",
            )
        );

        self.fut = Option::from(
            self.fut
                .as_ref()
                .unwrap()
                .call_method0(py, "__iter__")?
        );

        Ok(())
    }

    /// 
    /// Internal Method: AsyncServerRunner._iter_sleep() -> Option<PyObject>
    ///    
    ///     _iter_sleep is what actually yields the next iteration the future,
    ///     you could interprete this has `yield from` in python just with more
    ///     steps involved.
    /// 
    fn _iter_sleep(&mut self) -> Option<PyObject> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        // if the future isnt set we'll create a new one
        if self.fut.is_none() {
            let _ = self._sleep(py);
        }

        let nxt = self.fut
            .as_ref()
            .unwrap()
            .call_method0(py, "__next__");

        return match nxt {
            Ok(f) => Some(f),
            Err(_) => {
                self.server_state = 1;
                self.fut = None;

                None
            },
        }
    }
}

/// 
/// This implementation adds the required __await__ dunder for
/// python to use a coroutine, it just simply returns itself
/// 
#[pyproto]
impl PyAsyncProtocol for AsyncServerRunner {
    fn __await__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
}

///
/// This applies the iterator magic methods for producing a coroutine,
/// becauase asyncio is built entirely off of generators to give concurrency
/// we much build our objects top behaving like generators aswell, hence the use
/// of the IterProtocol.
/// 
#[pyproto]
impl PyIterProtocol for AsyncServerRunner {

    /// 
    /// Iter is just a required dunder so we return ourselves as the 
    /// iterator.
    /// 
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }

    /// 
    /// __next__ is what does all the actual work, this gets called every time by the event
    /// loop, Pyo3 gives us some useful helpers to replicate what yield and Return do.
    /// 
    /// If we yield it tells the event loop we still have something todo and we'll get called again
    /// this is useful if we plan on making a loop at anypoint because we cannot just do a standard 
    /// loop otherwise we would block.
    /// 
    /// when `server_state` is 0 we can use this to setup anything before yielding from another coro
    /// or in this case polling the server listener.
    /// 
    /// when `server_state` is 1 we poll our listener for a client or None, this is what is actually
    /// yielding everything other than if we set to state 2 where we sleep for x time.
    /// 
    fn __next__(mut slf: PyRefMut<Self>) -> PyResult<IterNextOutput<Option<PyObject>, Option<PyObject>>> {
        // setup futures
        if slf.server_state == 0 {
            slf.server_state = 1;
        }

        // yield futures
        if slf.server_state == 1 {
            let client = slf.server.accept_client();

            // if we have a client connecting we will get it as Some()
            if client.is_some() {

                // todo create task then parse stuff.
                let cli = client.unwrap();
                cli.set_nonblocking(true);
                let gil = Python::acquire_gil();
                let py = gil.python();
                let asyncio = py.import("asyncio")?;
                let caller = OnceFuture::new(Stream::new(cli));
                let _task = asyncio.call1( "ensure_future", (caller,))?;

                return Ok(IterNextOutput::Yield(None))
            }
            
            // Should we stop the server?
            if slf.server_exit {
                return Ok(IterNextOutput::Return(None))
            }

            // Lets change our sleep so we sleep for a bit
            slf.server_state = 2;
        }

        // Sleep x time (save cpu)
        if slf.server_state == 2 {
            return Ok(IterNextOutput::Yield(slf._iter_sleep()))
        }

        // Invalid state
        return Ok(IterNextOutput::Return(None))
    }
}


///
/// This struct is hell, litterally. It creates a 'false' Clone
/// implementation to allow Pyo3 to use it. This should NOT be allowed
/// to be accessible as a public interface because this ignores some required
/// functions like extract to convert pyobjects into it.
///
/// This is a one way Object only.
///
struct Stream {
    internal_stream: Option<TcpStream>
}

impl Stream {
    fn new(stream: TcpStream) -> Self {
        Self {
            internal_stream: Some(stream)
        }
    }
}

impl Clone for Stream {
    fn clone(&self) -> Self {
        // We'll just try clone this and hope it works
        // note: I have no idea what issues this can cause
        //       later on so...
        Self {
            internal_stream: Some(
                self.internal_stream
                    .as_ref()
                    .unwrap()
                    .try_clone()
                    .unwrap(),
            )
        }
    }
}

impl pyo3::conversion::FromPyObject<'_> for Stream {
    fn extract(_ob: &PyAny) -> PyResult<Self> {
        // Cant recreate a listener like this.
        Ok(Self {
            internal_stream: None
        })
    }
}


/// Wraps a Python future and TCP stream
#[pyclass]
struct OnceFuture {
    // External parameters
    stream: Stream,



    // Internals
    state: u8,

}

#[pymethods]
impl OnceFuture {
    #[new]
    fn new(stream: Stream) -> Self {
        OnceFuture {
            stream,
            state: 0,
        }
    }
}

#[pyproto]
impl PyAsyncProtocol for OnceFuture {
    fn __await__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
}

#[pyproto]
impl PyIterProtocol for OnceFuture {
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
    fn __next__(
        slf: PyRefMut<Self>) -> PyResult<IterNextOutput<Option<PyObject>, Option<PyObject>>> { // PyResult<IterNextOutput<Option<PyObject>, Option<(String, String, String, HashMap<String, String>)>>> {

        //let parsed = parse_partial(
        //    slf.stream.internal_stream.as_ref().unwrap())?;
        let _ = slf.stream.internal_stream.as_ref().unwrap().write_all(b"HTTP/1.1 200 OK\r\n\r\n");

        Ok(IterNextOutput::Return(None))
    }
}

#[pyclass]
#[derive(Debug)]
struct HTTPRequest {
    method: &'static str,
    path: &'static str,
    protocol: &'static str,
    headers: HashMap<String, String>,
}

///
/// Parses a tcp stream reading the headers; repeat until complete
/// todo: add a better parser
fn parse_partial(stream: &TcpStream) -> PyResult<(String, String, String, HashMap<String, String>)> {
    let mut reader = io::BufReader::new(stream);

    const MAX_HEADER_COUNT: usize = 32;

    let mut headers: HashMap<String, String> = HashMap::default();
    let mut method = String::new();
    let mut path= String::new();
    let mut protocol= String::new();

    for i in 0..MAX_HEADER_COUNT {
        let mut buff = Vec::with_capacity(1024);
        let n = reader.read_until(b'\n', &mut buff)?;
        let _ = buff.split_off(if n >= 2 {n-2} else {0});
        if &buff == b"" {
            break
        }

        if i != 0 {
            let mut iter = buff.splitn_str(2, b": ");
            headers.insert(
                String::from_utf8(
                    Vec::from(iter.next().unwrap())
                )?,
                String::from_utf8(
                    Vec::from(iter.next().unwrap().trim_start())
                )?
            );
        } else {
            let mut items =  buff.split_str( b" ").into_iter();

            method = String::from_utf8_lossy(items.next().unwrap()).parse()?;
            path = String::from_utf8_lossy(items.next().unwrap()).parse()?;
            protocol = String::from_utf8_lossy(items.next().unwrap()).parse()?;
        }
    }

    Ok((method, path, protocol, headers))
}


///
/// Wraps all our existing pyobjects together in the module
///
#[pymodule]
fn async_rust(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<AsyncServerRunner>()?;
    m.add_class::<OnceFuture>()?;
    Ok(())
}
