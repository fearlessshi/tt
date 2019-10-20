#![allow(non_snake_case)]
#![allow(unused_must_use)]
use std::thread;
use std::process;
use std::io::prelude::*;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

use tun::Device;
use tun::configure;
use tun::platform::linux;
use tun::platform::posix;
use crate::encoder::{Encoder};
use std::net::{self, Ipv4Addr, TcpStream};

pub fn setup(tun_ip: &str, BUFFER_SIZE: usize) -> (posix::Reader, posix::Writer){
    let mut conf = configure();
    conf.address(tun_ip);
    conf.netmask("255.255.255.0");
    conf.mtu((BUFFER_SIZE - 60) as i32);

    let mut iface = linux::create(&conf).unwrap_or_else(|err|{
        eprintln!("Failed to create tun device, {}", err);
        process::exit(-1);
    });

    iface.enabled(true).unwrap_or_else(|err|{
        eprintln!("Failed to enable tun device, {}", err);
        process::exit(-1);
    });

    iface.split()
}


//    pub fn handle_connection(&self, stream:net::TcpStream, encoder:Encoder) {
pub fn handle_connection(connection_rx: mpsc::Receiver<(TcpStream, Encoder)>, 
                        BUFFER_SIZE: usize, tun_ip: &str){

    let clients: HashMap<Ipv4Addr, (TcpStream, Encoder)> = HashMap::new();
    let clients = Arc::new(Mutex::new(clients));
    let (mut tun_reader, mut tun_writer) = setup(tun_ip, BUFFER_SIZE);

    let _clients = clients.clone();

    // thread: read from tun
    let _download = thread::spawn(move ||{
        let mut index: usize;
        let mut buf  = vec![0u8; BUFFER_SIZE];
        loop {
            // TODO what if we read less/more than a full IP packet ???
            index = match tun_reader.read(&mut buf[..BUFFER_SIZE-60]) {
                Ok(read_size) if read_size > 0 => read_size,
                _ => break
            };
            let dst_ip = Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]);
            if let Some((stream, encoder)) = _clients.lock().unwrap().get(&dst_ip) {
                index = encoder.encode(&mut buf, index);
                // TODO need a better solution
                // fix1: use non-blocking or seperate threads for each client
                // fix2: try not to copy the stream each time.
                let mut stream_write = stream.try_clone().unwrap();
                match stream_write.write(&buf[..index]) {
                    Ok(_) => continue,
                    Err(_) => continue,   // if client has disconnected, continue
                };
            }
        }
        //println!("Download stream exited...");
    });

    // thread: write to tun
    let (tun_tx, tun_rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move||{
        for i in tun_rx {
            tun_writer.write(&i);
        }
    });

    for (mut stream, encoder) in connection_rx {
        // thread: accept connection and write to channel
        stream.set_nodelay(true);
        let _clients = clients.clone();
        let _tun_tx = tun_tx.clone();
        let _upload = thread::spawn(move || {
            println!("got client connection");
            stream.set_nodelay(true);
            let mut index: usize = 0;
            let mut offset:i32 = 1 + 12 + 2 + 16;               // read least data at first
            let mut buf  = vec![0u8; BUFFER_SIZE];
            let decoder = encoder.clone();
            let mut stream_read = stream.try_clone().unwrap();

            // get destination ip from first packet
            loop {                                              // make sure read only one encrypted block
                index += match stream_read.read(&mut buf[index .. offset as usize]) {
                    Ok(read_size) if read_size > 0 => read_size,
                    _ => break,
                };

                let (data_len, _offset) = encoder.decode(&mut buf[..index]);
                offset = _offset;
                if data_len > 0 {
                    let data = &buf[offset as usize - data_len .. offset as usize];
                    match _tun_tx.send(Vec::from(data)) {
                        Ok(_) => (),
                        Err(_) => break
                    };

                    if data[0] == 0x44 {       // got special 'ipv4 handshake' packet
                        let src_ip = Ipv4Addr::new(data[1], data[2], data[3], data[4]);
                        _clients.lock().unwrap().insert(src_ip, (stream_read, encoder));
                        break;
                    }
                    else if data[0] >> 4 == 0x4 {        // got an ipv4 packet, cool
                        let src_ip = Ipv4Addr::new(data[12], data[13], data[14], data[15]);
                        _clients.lock().unwrap().insert(src_ip, (stream_read, encoder));
                        break;
                    }
                    else if data[0] == 0x66 {       // got special 'ipv6 handshake' packet
                        //
                    }
                    else if data[0] >> 4 == 0x6 {   // got an ipv6 packet, have to do another round
                        index = 0;
                        offset = 1 + 12 + 2 + 16;
                        continue;
                    }
                }
                else if offset > 0 {        // left_shall_be_read
                    offset = index as i32 + offset;
                }
                else if offset == -1 {
                    eprintln!("first packet decode error!");
                    return;
                }
                else { break; } // decrypted_size ==0 && offset == 0: not enough data to decode
            }

            println!("first packet process done");

            index = 0;
            loop {
                // from docs, size = 0 means EOF, 
                // maybe we don't need to worry about TCP Keepalive here.
                index += match stream.read(&mut buf[index..]) {
                    Ok(read_size) if read_size > 0 => read_size,
                    _ => break,
                };
                offset = 0;
                loop {
                    let (data_len, _offset) = decoder.decode(&mut buf[offset as usize..index]);
                    if data_len > 0 {
                        offset += _offset;
                        let data = &buf[offset as usize - data_len .. offset as usize];
                        match _tun_tx.send(Vec::from(data)) {
                            Ok(_) => (),
                            Err(_) => break
                        };
                        if (index - offset as usize) < (1 + 12 + 2 + 16) {
                            break; // definitely not enough data to decode
                        }
                    }
                    else if _offset == -1 {
                        eprintln!("upload stream decode error!");
                        offset = -1;
                        break;
                    }
                    else { break; } // decrypted_size ==0 && offset == 0: not enough data to decode
                }
                if offset == -1 {break;}
                buf.copy_within(offset as usize .. index, 0);
                index = index - (offset as usize);
            }
            stream.shutdown(net::Shutdown::Both);
            //println!("Upload stream exited...");
        });
    }
}
