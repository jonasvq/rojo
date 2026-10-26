use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(
        args.len(),
        1,
        "Studio must receive one unmodified place path"
    );
    assert!(std::path::Path::new(&args[0]).is_file());
    let port: u16 = std::env::var("ROJO_OPEN_TEST_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET /api/rojo HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200"));
    std::fs::write(
        std::env::var_os("ROJO_OPEN_TEST_LOG").unwrap(),
        format!("{}\nHTTP_200\n", args[0]),
    )
    .unwrap();
}
