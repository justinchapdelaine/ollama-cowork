use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};

/// Allocates loopback-only ports. The launched service must still fail closed if
/// another process wins the bind before startup completes.
pub trait LoopbackPortAllocator: Send {
    fn allocate(&mut self) -> Result<u16, String>;
}

#[derive(Default)]
pub struct SystemLoopbackPortAllocator;

impl LoopbackPortAllocator for SystemLoopbackPortAllocator {
    fn allocate(&mut self) -> Result<u16, String> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("could not allocate a loopback port: {error}"))?;
        listener
            .local_addr()
            .map(|address| address.port())
            .map_err(|error| format!("could not inspect allocated loopback port: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_allocator_returns_bindable_nonzero_loopback_ports() {
        let port = SystemLoopbackPortAllocator.allocate().unwrap();
        assert_ne!(port, 0);
        TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    }
}
