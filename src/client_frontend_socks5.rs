#![allow(non_snake_case)]
#![allow(unused_must_use)]
use std::net;
use std::thread;
use crate::client;
use std::io::prelude::*;

pub fn run(KEY:&'static str, SERVER_ADDR:&'static str, BIND_ADDR:&'static str, 
                        PORT_RANGE_START:u32, PORT_RANGE_END:u32, MTU:usize) {

    let listener = match net::TcpListener::bind(BIND_ADDR){
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("Failed to bind [{}], {}", BIND_ADDR, err);
            return;
        }
    };
    for stream in listener.incoming() {
        thread::spawn(move||{
            handle_connection(stream.unwrap(),
                KEY, SERVER_ADDR, PORT_RANGE_START, PORT_RANGE_END, MTU);
        });
    }
}

pub fn handle_connection(local_stream:net::TcpStream, KEY:&'static str, SERVER_ADDR:&'static str,
                        PORT_RANGE_START:u32, PORT_RANGE_END:u32, MTU:usize) {
    let (upstream, encoder) = match client::get_stream(KEY, SERVER_ADDR, PORT_RANGE_START, PORT_RANGE_END) {
        Ok((upstream, encoder)) => (upstream, encoder),
        Err(err) => {
            eprintln!("Error: Failed to connect to server, {}", err);
            return;
        }
    };

    local_stream.set_nodelay(true);
    upstream.set_nodelay(true);

    let mut upstream_read = upstream.try_clone().unwrap();
    let mut upstream_write = upstream.try_clone().unwrap();
    let mut local_stream_read = local_stream.try_clone().unwrap();
    let mut local_stream_write = local_stream.try_clone().unwrap();
    let decoder = encoder.clone();

    // download stream
    let _download = thread::spawn(move || {
        //std::io::copy(&mut upstream_read, &mut local_stream_write);
        let mut index: usize = 0;
        let mut buf = vec![0u8; MTU];
        let mut buf2 = vec![0u8; MTU];
        loop {
            index += match upstream_read.read(&mut buf[index..]) {
                Ok(read_size) if read_size > 0 => read_size,
                _ => {
                    //eprintln!("upstream read failed");
                    upstream_read.shutdown(net::Shutdown::Both);
                    local_stream_write.shutdown(net::Shutdown::Both);
                    // #TODO
                    // 1. distinguish from server port close, something like a packet "FFFF"..
                    // 2. upstream status shall be handled by client.rs, encode/decode included
                    break;
                }
            };
            let (decrypted_size, offset) = decoder.decode(&buf[..index], &mut buf2);
            if decrypted_size > 0 {
                match local_stream_write.write(&buf2[..decrypted_size]) {
                    Ok(_) => (),
                    _ => {
                        //eprintln!("local_stream write failed");
                        upstream_read.shutdown(net::Shutdown::Both);
                        local_stream_write.shutdown(net::Shutdown::Both);
                        break;
                    }
                };

                buf.copy_within(offset as usize .. index, 0);
                index = index - (offset as usize);
            }
            else if offset == -1 {
                 eprintln!("download stream decode error!");
            }
            else {} // decrypted_size ==0 && offset == 0: packet length not ok
        }
        //println!("Download stream exited...");
    });

    // upload stream
    let _upload = thread::spawn(move || {
        //std::io::copy(&mut local_stream_read, &mut upstream_write);
        let mut index: usize;
        let mut buf = vec![0u8;  MTU-50];
        let mut buf2 = vec![0u8; MTU];
        loop {
            // from docs, size = 0 means EOF,
            // maybe we don't need to worry about TCP Keepalive here.
            index = match local_stream_read.read(&mut buf) {
                Ok(read_size) if read_size > 0 => read_size,
                _ => {
                    //eprintln!("local_stream read failed");
                    upstream_write.shutdown(net::Shutdown::Both);
                    local_stream_read.shutdown(net::Shutdown::Both);
                    break;
                }
            };
            index = encoder.encode(&buf[..index], &mut buf2);
            match upstream_write.write(&buf2[..index]) {
                Ok(_) => (),
                _ => {
                    //eprintln!("upstream write failed");
                    upstream_write.shutdown(net::Shutdown::Both);
                    break;
                }
            };
        }
        //println!("Upload stream exited...");
    });
}
