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
fn get_loop(py: Python) -> PyResult<&PyAny> {
    let asyncio = py.import("asyncio")?;
    Ok(asyncio.call0("get_event_loop")?)
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
    loop_: PyObject,
    fut: Option<Py<PyAny>>,
}

#[pymethods]
impl AsyncServerRunner {
    #[new]
    fn new(binding_addr: String) -> Self {
        println!("Connecting to {}", &binding_addr);

        let server = AsyncServer::new(binding_addr.clone());
        let loop_ = {
            let gil = Python::acquire_gil();
            let py = gil.python();
            get_loop(py).unwrap().into_py(py)
        };

        AsyncServerRunner {
            addr: binding_addr,
            server,
            server_state: 0,
            server_exit: false,
            loop_,
            fut: None,
        }
    }

    fn _sleep(&mut self, py: Python) -> PyResult<()> {
        self.fut = Option::from(self.loop_.call_method0(py, "create_future")?);

        let futures = py.import("asyncio")?.get("futures")?;
        let _ = self.loop_.call_method1(
            py,
            "call_later",
            (1, futures.getattr("_set_result_unless_cancelled")?, self.fut.as_ref(), "")
        );

        self.fut = Option::from(self.fut.as_ref().unwrap().call_method0(py, "__iter__")?);

        Ok(())
    }

    fn _iter_sleep(&mut self) -> Option<PyObject> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        // if the future isnt set we'll create a new one
        if self.fut.is_none() {
            self._sleep(py);
        }

        let nxt = self.fut
            .as_ref()
            .unwrap()
            .call_method0(py, "__next__");

        return match nxt {
            Ok(f) => Some(f),
            Err(er) => {
                self.server_state = 1;
                self.fut = None;

                None
            },
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


#[pymodule]
fn async_rust(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<AsyncServerRunner>()?;
    Ok(())
}