use pyo3::prelude::*;
use pyo3::PyIterProtocol;
use pyo3::class::pyasync::PyAsyncProtocol;
use pyo3::class::pyasync::PyAsyncAwaitProtocol;
use pyo3::exceptions::{
    PyStopIteration,
    PyAssertionError,
};
use pyo3::class::iter::IterNextOutput;
use std::net::{TcpListener, TcpStream, Incoming};
use std::error::Error;
use std::io;
use std::borrow::{Borrow, BorrowMut};
use pyo3::class::iter::IterNextOutput::Yield;

///
/// This is essentially the same as:
///
///
/// await asyncio.sleep(delay)
/// or
/// yield from asyncio.sleep(delay)
///
fn async_sleep(py: Python, delay: f64) -> PyResult<Py<PyAny>> {
    let asyncio = py.import("asyncio")?;
    Ok(
        asyncio
            .call1("sleep", (delay,))?
            .call_method0("__await__")?
            .into_py(py)
    )
}

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

#[pyclass]
struct AsyncServerRunner {
    addr: String,
    server: AsyncServer,
    server_state: u8,
    server_exit: bool,
    sleep_fut: Option<Py<PyAny>>,
}

#[pymethods]
impl AsyncServerRunner {
    #[new]
    fn new(binding_addr: String) -> Self {
        println!("Connecting to {}", &binding_addr);

        let server = AsyncServer::new(binding_addr.clone());
        AsyncServerRunner {
            addr: binding_addr,
            server,
            server_state: 0,
            server_exit: false,
            sleep_fut: None,
        }
    }
}

#[pyproto]
impl PyAsyncProtocol for AsyncServerRunner {
    fn __await__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
}

#[pyproto]
impl PyIterProtocol for AsyncServerRunner {
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
    fn __next__(mut slf: PyRefMut<Self>) -> PyResult<IterNextOutput<Option<PyObject>, Option<PyObject>>> {
        println!("call?");

        // setup futures
        if slf.server_state == 0 {

            slf.server_state = 1;
        }

        // yield futures
        if slf.server_state == 1 {
            let thing = slf.server.accept_client();
            println!("client {:?}", thing);



            if slf.server_exit {
                return Ok(IterNextOutput::Return(None))
            }
            slf.server_state = 2;
        }

        // Sleep x time (save cpu)
        if slf.server_state == 2 {
            let gil = Python::acquire_gil();
            if slf.sleep_fut.is_some() {
                let py = gil.python();
                slf.sleep_fut = Some(async_sleep(py, 0.1)?);
            }

            let py = gil.python();
            let nxt = slf.sleep_fut
                .as_ref()
                .unwrap()
                .call_method0(py, "__next__")?;

            return Ok(IterNextOutput::Yield(None))
        }

        // Invalid state
        return Ok(IterNextOutput::Return(None))
    }
}


#[pymodule]
fn async_rust(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<AsyncServerRunner>()?;
    Ok(())
}