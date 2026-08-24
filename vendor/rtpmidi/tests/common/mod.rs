use std::net::UdpSocket;

pub fn find_consecutive_ports() -> (u16, u16) {
    loop {
        let socket = UdpSocket::bind(("0.0.0.0", 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        let next_port = port + 1;
        if let Ok(socket2) = UdpSocket::bind(("0.0.0.0", next_port)) {
            drop(socket);
            drop(socket2);
            return (port, next_port);
        }
    }
}
