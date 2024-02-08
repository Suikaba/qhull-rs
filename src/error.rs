use std::{error::Error, fmt::Display};

use crate::{helpers::QhTypeRef, sys, tmp_file::TmpFile, Face, Ridge, Vertex};

macro_rules! define_error_kinds {
    (
        $(
            $(#[$attr:meta])*
            $name:ident => $code:literal,
        ),*$(,)?
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum QhErrorKind {
            $(
                $(#[$attr])*
                ///
                #[doc = concat!("Error code ", $code)]
                $name,
            )*

            /// An error code that is not part of the enum.
            Other(i32),
        }

        impl QhErrorKind {
            pub fn from_code(code: i32) -> Self {
                match code {
                    0 => panic!("0 is not an error code"),
                    $(
                        $code => Self::$name,
                    )*
                    _ => Self::Other(code),
                }
            }
            pub fn error_code(&self) -> i32 {
                match self {
                    $(
                        Self::$name => $code,
                    )*
                    Self::Other(code) => *code,
                }
            }
        }
    };
}

define_error_kinds! {
    // TODO ...
}

#[derive(Debug, Clone)]
pub struct QhError<'a> {
    pub kind: QhErrorKind,
    pub error_message: Option<String>,
    pub face: Option<Face<'a>>,
    pub ridge: Option<Ridge<'a>>,
    pub vertex: Option<Vertex<'a>>,
}

impl<'a> Display for QhError<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Qhull error: {:?} (#{})",
            self.kind,
            self.kind.error_code()
        )?;
        if let Some(msg) = &self.error_message {
            write!(f, "\n{}", msg)?;
        }
        if let Some(face) = &self.face {
            write!(f, "\nFace: {:?}", face)?;
        }
        if let Some(ridge) = &self.ridge {
            write!(f, "\nRidge: {:?}", ridge)?;
        }
        if let Some(vertex) = &self.vertex {
            write!(f, "\nVertex: {:?}", vertex)?;
        }
        Ok(())
    }
}

impl<'a> Error for QhError<'a> {}

impl<'a> QhError<'a> {
    pub fn into_static(self) -> QhError<'static> {
        let QhError {
            kind,
            error_message,
            face,
            ridge,
            vertex,
        } = self;
        if let Some(face) = face {
            eprintln!(
                "During conversion to static, a face was discarded: {:?}",
                face
            );
        }
        if let Some(ridge) = ridge {
            eprintln!(
                "During conversion to static, a ridge was discarded: {:?}",
                ridge
            );
        }
        if let Some(vertex) = vertex {
            eprintln!(
                "During conversion to static, a vertex was discarded: {:?}",
                vertex
            );
        }
        QhError {
            kind,
            error_message,
            face: None,
            ridge: None,
            vertex: None,
        }
    }

    /// Try to run a function on a raw qhT instance and handle errors.
    ///
    /// # Safety
    /// * shall not be nested
    /// * shall not be called when errors are already being handled
    ///
    /// # Implementation details
    ///
    /// Qhull uses [`setjmp`/`longjmp`](https://en.cppreference.com/w/c/program/longjmp) for error handling, this is not currently supported in Rust.
    /// For this reason, the actual error handling is done in C and this function is just a wrapper around the C function [`qhull_sys__try_on_qh`](sys::qhull_sys__try_on_qh).
    ///
    /// Relevant links:
    /// - <https://github.com/rust-lang/rfcs/issues/2625>: RFC for adding support for `setjmp`/`longjmp` to Rust, describes the current problems with `setjmp`/`longjmp` in Rust.
    /// - <https://docs.rs/setjmp/0.1.4/setjmp/index.html>
    /// - <https://en.cppreference.com/w/c/program/longjmp>
    /// - <https://learn.microsoft.com/en-en/cpp/cpp/using-setjmp-longjmp?view=msvc-170>
    /// - <http://groups.di.unipi.it/~nids/docs/longjump_try_trow_catch.html>
    pub unsafe fn try_on_raw<'b, R, F>(
        qh: &mut sys::qhT,
        err_file: &mut Option<TmpFile>,
        f: F,
    ) -> Result<R, QhError<'b>>
    where
        F: FnOnce(&mut sys::qhT) -> R,
    {
        unsafe extern "C" fn cb<F2>(qh: *mut sys::qhT, data: *mut std::ffi::c_void)
        where
            F2: FnOnce(&mut sys::qhT),
        {
            assert!(!qh.is_null(), "qh is null");
            assert!(!data.is_null(), "data is null");
            let qh = &mut *qh;
            let f: &mut Option<F2> = &mut *(data as *mut _);
            f.take().unwrap()(qh);
        }

        fn get_cb<F>(
            _: &mut Option<F>,
        ) -> unsafe extern "C" fn(*mut sys::qhT, *mut std::ffi::c_void)
        where
            F: FnOnce(&mut sys::qhT),
        {
            cb::<F>
        }

        let mut result = None;

        let mut f = Some(|qh: &mut sys::qhT| result = Some(f(qh)));

        let err_code = unsafe {
            sys::qhull_sys__try_on_qh(
                &mut *qh,
                Some(get_cb(&mut f)),
                &mut f as *mut _ as *mut std::ffi::c_void,
            )
        };

        if err_code == 0 {
            Ok(result.unwrap())
        } else {
            let kind = QhErrorKind::from_code(err_code);
            let file = err_file
                .replace(TmpFile::new().expect("Failed to create a replacement temporary file"));
            qh.ferr = err_file.as_ref().unwrap().file_handle();
            let msg = file.map(|file| file.read_as_string_and_close().unwrap());
            Err(QhError {
                kind,
                error_message: msg,
                face: Face::from_ptr(qh.tracefacet, qh.input_dim as _), // TODO is this dim correct?
                ridge: Ridge::from_ptr(qh.traceridge, qh.input_dim as _), // TODO is this dim correct?
                vertex: Vertex::from_ptr(qh.tracevertex, qh.input_dim as _), // TODO is this dim correct?
            })
        }
    }
}
