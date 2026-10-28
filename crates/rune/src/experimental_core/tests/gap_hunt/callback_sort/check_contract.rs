fn drain_replay(mut pipe: impl Read, exceeded: Arc<AtomicBool>) -> Vec<u8> {
	let mut result = Vec::new();
	let mut buffer = [0u8; 4096];
	loop {
		let count = pipe.read(&mut buffer).unwrap();
		if count == 0 {
			break;
		}
		if result.len() + count > CAP {
			exceeded.store(true, Ordering::SeqCst);
		} else {
			result.extend_from_slice(&buffer[..count]);
		}
	}
	result
}
