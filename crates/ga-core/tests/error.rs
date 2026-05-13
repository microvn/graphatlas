use ga_core::Error;

#[test]
fn jsonrpc_codes_match_as023_table() {
    assert_eq!(
        Error::IndexNotReady {
            status: "indexing".into(),
            progress: 0.4
        }
        .jsonrpc_code(),
        -32000
    );
    assert_eq!(
        Error::ParseError {
            file: "a.py".into(),
            lang: "python".into(),
            err: "x".into()
        }
        .jsonrpc_code(),
        -32001
    );
    assert_eq!(
        Error::ConfigCorrupt {
            path: "/tmp/x".into(),
            reason: "bad json".into()
        }
        .jsonrpc_code(),
        -32002
    );
    assert_eq!(
        Error::SchemaVersionMismatch {
            cache: 1,
            binary: 2
        }
        .jsonrpc_code(),
        -32003
    );
    assert_eq!(
        Error::Database("connection refused".into()).jsonrpc_code(),
        -32005
    );
}

#[test]
fn io_error_converts() {
    let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    let e: Error = io.into();
    assert_eq!(e.jsonrpc_code(), -32004);
}

#[test]
fn display_is_informative() {
    let e = Error::SchemaVersionMismatch {
        cache: 1,
        binary: 2,
    };
    let s = format!("{e}");
    assert!(s.contains("cache=1"));
    assert!(s.contains("binary=2"));
}
