//! A minimum echo server.
//!
//! Run `cargo run --example echo -- --help` to get usage message.

mod echo {
    // All this module are runtime agnostic.
    use async_net::*;
    use futures_lite::io;

    pub async fn echo_server(port: u16) {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        println!("Listen on port: {}", listener.local_addr().unwrap().port());
        let mut echos = vec![];
        let mut id_counter = 0;
        loop {
            let (stream, remote_addr) = listener.accept().await.unwrap();
            id_counter += 1;
            let id = id_counter;
            let handle = spawns::spawn(async move {
                eprintln!("{id:010}[{remote_addr}]: serving");
                let (reader, writer) = io::split(stream);
                match io::copy(reader, writer).await {
                    Ok(_) => eprintln!("{id:010}[{remote_addr}]: closed"),
                    Err(err) => eprintln!("{id:010}[{remote_addr}]: {err:?}"),
                }
            })
            .attach();
            echos.push(handle);
        }
    }
}

fn main() {
    use clap::*;
    let cmd = Command::new("echo")
        .arg(
            Arg::new("port")
                .long("port")
                .help("Port to listen. Defaults to 0 to choose an random one")
                .value_parser(value_parser!(u16)),
        )
        .arg(
            Arg::new("parallelism")
                .long("parallelism")
                .help("Concurrent parallelism. Defaults to 0 to utilize all cores")
                .value_parser(value_parser!(usize)),
        );

    let matches = cmd.get_matches();
    let port = matches.get_one::<u16>("port").copied().unwrap_or(0);
    let parallelism = matches
        .get_one::<usize>("parallelism")
        .copied()
        .unwrap_or(0);
    spawns::Blocking::new(parallelism).block_on(echo::echo_server(port));
}
