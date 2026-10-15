// seq14985: storage compaction only. Equality covers the complete canonical bytes.
const C10_LARGE: &[&str] = &[
	"visits/plain/65534",
	"visits/plain/65535",
	"visits/plain/65536",
	"visits/result/65533",
	"visits/result/65534",
	"visits/result/65535",
	"large/key-1MiB",
];
fn c10_canonical(value: &Json) -> Vec<u8> {
	fn ordered(value: &Json) -> Json {
		match value {
			Json::Object(map) => {
				let mut keys = map.keys().collect::<Vec<_>>();
				keys.sort();
				let mut out = serde_json::Map::new();
				for key in keys {
					out.insert(key.clone(), ordered(&map[key]));
				}
				Json::Object(out)
			}
			Json::Array(values) => Json::Array(values.iter().map(ordered).collect()),
			_ => value.clone(),
		}
	}
	serde_json::to_vec(&ordered(value)).unwrap()
}
fn c10_sha(data: &[u8]) -> String {
	let mut child = Command::new("/usr/bin/sha256sum")
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	child.stdin.take().unwrap().write_all(data).unwrap();
	let out = child.wait_with_output().unwrap();
	assert!(out.status.success() && out.stderr.is_empty());
	let text = String::from_utf8(out.stdout).unwrap();
	let hash = text.strip_suffix("  -\n").unwrap();
	assert_eq!(hash.len(), 64);
	assert!(hash
		.bytes()
		.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
	hash.into()
}
fn c10_hex_bytes(text: &str) -> Result<Vec<u8>, String> {
	if !text.len().is_multiple_of(2) {
		return Err("odd hex length".into());
	}
	(0..text.len())
		.step_by(2)
		.map(|at| u8::from_str_radix(&text[at..at + 2], 16).map_err(|_| "invalid hex".into()))
		.collect()
}
fn c10_decode(text: &str) -> Json {
	serde_json::from_slice(&c10_hex_bytes(text).unwrap()).unwrap()
}
fn c10_edges(node: &Json) -> Result<Vec<usize>, String> {
	let arr = node.as_array().ok_or("graph node is not array")?;
	let tag = arr
		.first()
		.and_then(Json::as_str)
		.ok_or("missing node tag")?;
	let ids: Vec<&Json> = match tag {
		"Vec" | "Tuple" => arr[2]
			.as_array()
			.ok_or("invalid sequence children")?
			.iter()
			.collect(),
		"Object" => arr[2]
			.as_array()
			.ok_or("invalid object fields")?
			.iter()
			.map(|f| &f[1])
			.collect(),
		"Option" => {
			if arr[2].is_null() {
				vec![]
			} else {
				vec![&arr[2]]
			}
		}
		"Result" => vec![&arr[3]],
		_ => vec![],
	};
	ids.into_iter()
		.map(|id| {
			id.as_u64()
				.and_then(|n| usize::try_from(n).ok())
				.ok_or_else(|| "invalid graph child".into())
		})
		.collect()
}
fn c10_summary_record(node: &Json) -> Result<Json, String> {
	let mut result = node.clone();
	let tag = node[0].as_str().ok_or("node tag")?;
	if matches!(tag, "Vec" | "Tuple") && node[2].as_array().ok_or("sequence edges")?.len() > 8 {
		let edges = node[2].as_array().unwrap();
		let bytes = c10_canonical(&node[2]);
		let n = edges.len();
		result[2] = json!({"encoding":"edges-sha256","count":n,"bytes":bytes.len(),"sha256":c10_sha(&bytes),"first4":&edges[..4],"last4":&edges[n-4..]});
	}
	if tag == "Object" {
		for field in result[2].as_array_mut().ok_or("object fields")? {
			let key = c10_hex_bytes(field[0].as_str().ok_or("object key")?)?;
			if key.len() > 64 {
				field[0] =
					json!({"encoding":"key-sha256","bytes":key.len(),"sha256":c10_sha(&key)});
			}
		}
	}
	Ok(result)
}
fn c10_summary(common: &Json) -> Result<Json, String> {
	let graph = &common["outcome"]["graph"];
	let nodes = graph["nodes"].as_array().ok_or("missing graph nodes")?;
	let root = graph["root"]
		.as_u64()
		.and_then(|n| usize::try_from(n).ok())
		.ok_or("invalid graph root")?;
	if root >= nodes.len() {
		return Err("graph root out of range".into());
	}
	let edges = nodes.iter().map(c10_edges).collect::<Result<Vec<_>, _>>()?;
	let mut indegree = vec![0usize; nodes.len()];
	for list in &edges {
		for &child in list {
			if child >= nodes.len() {
				return Err("child out of range".into());
			}
			indegree[child] += 1;
		}
	}
	let mut queue = indegree
		.iter()
		.enumerate()
		.filter_map(|(i, &n)| (n == 0).then_some(i))
		.collect::<Vec<_>>();
	let mut depth = vec![-1isize; nodes.len()];
	depth[root] = 0;
	let mut at = 0;
	while at < queue.len() {
		let parent = queue[at];
		at += 1;
		for &child in &edges[parent] {
			if depth[parent] >= 0 {
				depth[child] = depth[child].max(depth[parent] + 1);
			}
			indegree[child] -= 1;
			if indegree[child] == 0 {
				queue.push(child);
			}
		}
	}
	if queue.len() != nodes.len() {
		return Err("summary cycle".into());
	}
	let first = nodes
		.iter()
		.take(4)
		.map(c10_summary_record)
		.collect::<Result<Vec<_>, _>>()?;
	let last = nodes[nodes.len().saturating_sub(4)..]
		.iter()
		.map(c10_summary_record)
		.collect::<Result<Vec<_>, _>>()?;
	let mut summary = json!({"node_count":nodes.len(),"max_depth":depth.iter().max().copied().unwrap_or(0),"root_type":nodes[root][0],"first4":first,"last4":last});
	let mut keys = Vec::new();
	for node in nodes {
		if node[0] == "Object" {
			for field in node[2].as_array().ok_or("object fields")? {
				let hex = field[0].as_str().ok_or("object key")?;
				if hex.len() > 128 {
					let key = c10_hex_bytes(hex)?;
					keys.push(json!({"bytes":key.len(),"sha256":c10_sha(&key)}));
				}
			}
		}
	}
	if !keys.is_empty() {
		summary["large_keys"] = Json::Array(keys);
	}
	Ok(summary)
}
fn c10_compare(actual: &Json, stored: &Json, label: &str) -> Result<(), String> {
	if stored["encoding"] == "sha256-common-v1" {
		if !C10_LARGE.contains(&label) {
			return Err("unlisted digest label".into());
		}
		let bytes = c10_canonical(actual);
		let hash = c10_sha(&bytes);
		if stored["bytes"].as_u64() != Some(bytes.len() as u64) || stored["sha256"] != hash {
			return Err(format!("full canonical equality failed for {label}; expected bytes={} sha={} summary={}; actual bytes={} sha={} summary={}",
				stored["bytes"],stored["sha256"],stored["summary"],bytes.len(),hash,c10_summary(actual)?));
		}
		if c10_summary(actual)? != stored["summary"] {
			return Err("digest diagnostic summary drift".into());
		}
		Ok(())
	} else if actual == stored {
		Ok(())
	} else {
		Err(format!(
			"full common equality failed for {label}: expected={stored}; actual={actual}"
		))
	}
}
