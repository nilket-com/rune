//! One blocking jobserver for all native, Cargo and interpreter work.
use anyhow::{ensure, Context as _, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::process::Command;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Instant;

#[derive(Serialize)]
pub struct Event {
	pub label: String,
	pub slots: usize,
	pub start_seconds: f64,
	pub end_seconds: f64,
}

type Token = std::io::Result<jobserver::Acquired>;
#[derive(Default)]
struct Requests {
	aborted: bool,
	pending: VecDeque<mpsc::Sender<Token>>,
}

#[derive(Default)]
struct Admission {
	next_ticket: usize,
	waiting: VecDeque<(usize, usize)>,
	reserved: usize,
	wide_waiters: usize,
	aborted: bool,
}

pub struct Budget {
	requests: Arc<Mutex<Requests>>,
	helper: jobserver::HelperThread,
	client: jobserver::Client,
	capacity: usize,
	// Reserve a whole width before touching the pipe. A wide waiter never
	// hoards partial tokens while long interpreter jobs are still running.
	admission: Mutex<Admission>,
	released: Condvar,
	events: Mutex<Vec<Event>>,
	epoch: Instant,
}

pub struct Permit {
	budget: Arc<Budget>,
	tokens: Vec<jobserver::Acquired>,
	label: String,
	start: Instant,
}

impl Budget {
	pub fn new(capacity: usize) -> Result<Arc<Self>> {
		ensure!(capacity > 0, "CPU budget must be positive");

		let client = jobserver::Client::new(capacity)?;
		let requests = Arc::new(Mutex::new(Requests::default()));
		let callback = requests.clone();
		let helper = client.clone().into_helper_thread(move |token| {
			if let Some(recipient) = callback.lock().unwrap().pending.pop_front() {
				// A canceled waiter drops its token instead of leaking it.
				let _ = recipient.send(token);
			}
		})?;
		Ok(Arc::new(Self {
			client,
			requests,
			helper,
			capacity,
			admission: Mutex::new(Admission::default()),
			released: Condvar::new(),
			events: Mutex::new(Vec::new()),
			epoch: Instant::now(),
		}))
	}
	pub fn capacity(&self) -> usize {
		self.capacity
	}
	pub fn acquire(self: &Arc<Self>, label: &str, slots: usize) -> Result<Permit> {
		self.acquire_range(label, slots, slots)
	}
	/// Native libraries can start with currently free capacity instead of
	/// waiting for every interpreter to finish. Exclusive work stays exact.
	pub fn acquire_up_to(self: &Arc<Self>, label: &str, maximum: usize) -> Result<Permit> {
		self.acquire_range(label, 1, maximum)
	}
	fn acquire_range(
		self: &Arc<Self>,
		label: &str,
		minimum: usize,
		maximum: usize,
	) -> Result<Permit> {
		ensure!(
			minimum > 0 && minimum <= maximum && maximum <= self.capacity,
			"invalid CPU reservation"
		);

		let mut state = self
			.admission
			.lock()
			.map_err(|_| anyhow::anyhow!("admission lock poisoned"))?;
		let ticket = state.next_ticket;
		state.next_ticket = state
			.next_ticket
			.checked_add(1)
			.context("admission ticket overflow")?;
		state.waiting.push_back((ticket, minimum));
		if minimum > 1 {
			state.wide_waiters += 1;
		}
		self.released.notify_all();
		// FIFO among requests that fit. An unfit whole-machine reservation
		// cannot create a barrier over smaller ready work.
		while !state.aborted
			&& state
				.waiting
				.iter()
				.find(|(_, width)| *width <= self.capacity - state.reserved)
				.map(|(id, _)| *id)
				!= Some(ticket)
		{
			state = self
				.released
				.wait(state)
				.map_err(|_| anyhow::anyhow!("admission lock poisoned"))?;
		}
		let position = state
			.waiting
			.iter()
			.position(|(id, _)| *id == ticket)
			.context("lost admission request")?;
		state.waiting.remove(position);
		if minimum > 1 {
			state.wide_waiters -= 1;
		}
		ensure!(!state.aborted, "CPU budget canceled after failed build");
		let slots = maximum.min(self.capacity - state.reserved);
		state.reserved += slots;
		self.released.notify_all();
		drop(state);
		let acquired = (|| -> Result<Vec<jobserver::Acquired>> {
			let mut tokens = Vec::with_capacity(slots);
			for _ in 0..slots {
				let (sender, receiver) = mpsc::channel();
				{
					let mut requests = self
						.requests
						.lock()
						.map_err(|_| anyhow::anyhow!("token request lock poisoned"))?;
					ensure!(!requests.aborted, "CPU budget canceled after failed build");
					requests.pending.push_back(sender);
					self.helper.request_token();
				}
				tokens.push(receiver.recv().context("CPU token request lost")??);
			}
			Ok(tokens)
		})();
		let tokens = match acquired {
			Ok(tokens) => tokens,
			Err(error) => {
				self.release_admission(slots);
				return Err(error);
			}
		};
		Ok(Permit {
			budget: self.clone(),
			tokens,
			label: label.to_owned(),
			start: Instant::now(),
		})
	}
	/// Wake blocked launchers after a failed Cargo child may have lost tokens.
	/// No replacement tokens are manufactured, and no new work can start.
	pub fn abort(&self) {
		{
			let mut state = self.admission.lock().unwrap();
			state.aborted = true;
			self.released.notify_all();
		}
		let mut requests = self.requests.lock().unwrap();
		requests.aborted = true;
		for sender in requests.pending.drain(..) {
			let _ = sender.send(Err(std::io::Error::new(
				std::io::ErrorKind::Interrupted,
				"verification CPU budget canceled",
			)));
		}
	}

	fn release_admission(&self, slots: usize) {
		let mut state = self.admission.lock().unwrap();
		state.reserved -= slots;
		self.released.notify_all();
	}

	pub fn configure(&self, command: &mut Command) {
		self.client.configure(command);
	}
	pub fn events(&self) -> Result<serde_json::Value> {
		let events = self
			.events
			.lock()
			.map_err(|_| anyhow::anyhow!("CPU event lock poisoned"))?;
		Ok(
			serde_json::json!({"capacity":self.capacity,"events":*events,"method":"blocking jobserver reservations; child process wait; no status polling"}),
		)
	}
}
impl Permit {
	pub fn slots(&self) -> usize {
		self.tokens.len()
	}
}
impl Drop for Permit {
	fn drop(&mut self) {
		let end = Instant::now();
		if let Ok(mut events) = self.budget.events.lock() {
			events.push(Event {
				label: self.label.clone(),
				slots: self.tokens.len(),
				start_seconds: self.start.duration_since(self.budget.epoch).as_secs_f64(),
				end_seconds: end.duration_since(self.budget.epoch).as_secs_f64(),
			});
		}
		let slots = self.tokens.len();
		self.tokens.clear();
		self.budget.release_admission(slots);
	}
}

/// Preserve input order while independent ready jobs execute concurrently.
pub fn map<T: Sync, U: Send>(
	items: &[T],
	workers: usize,
	f: impl Fn(&T) -> Result<U> + Sync,
) -> Result<Vec<U>> {
	ensure!(workers > 0, "worker pool must be positive");
	let next = std::sync::atomic::AtomicUsize::new(0);
	let rows = Mutex::new(Vec::new());
	std::thread::scope(|scope| -> Result<()> {
		let mut handles = Vec::new();
		for _ in 0..workers.min(items.len()) {
			let (next, rows, f) = (&next, &rows, &f);
			handles.push(scope.spawn(move || loop {
				let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
				let Some(item) = items.get(index) else { break };
				let result = f(item);
				rows.lock().unwrap().push((index, result));
			}));
		}
		let mut panicked = false;
		for handle in handles {
			if handle.join().is_err() {
				panicked = true;
			}
		}
		ensure!(!panicked, "job worker panicked");
		Ok(())
	})?;
	let mut rows = rows
		.into_inner()
		.map_err(|_| anyhow::anyhow!("job result lock poisoned"))?;
	ensure!(rows.len() == items.len(), "incomplete job graph");
	rows.sort_by_key(|row| row.0);
	rows.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn independent_jobs_overlap_and_results_remain_ordered() {
		let barrier = std::sync::Barrier::new(2);
		let rows = map(&[1, 2], 2, |x| {
			barrier.wait();
			Ok(*x)
		})
		.unwrap();
		assert_eq!(rows, vec![1, 2]);
	}
	#[test]
	fn worker_failures_and_panics_never_become_passes() {
		assert!(map(&[1, 2], 2, |x| {
			ensure!(*x != 2, "failed witness");
			Ok(*x)
		})
		.is_err());
		assert!(map(&[1, 2], 2, |x| {
			assert_ne!(*x, 2, "injected panic");
			Ok(*x)
		})
		.is_err());
	}
	#[test]
	fn wide_reservations_release_without_deadlock() {
		let budget = Budget::new(2).unwrap();
		std::thread::scope(|scope| {
			for _ in 0..3 {
				let budget = &budget;
				scope.spawn(move || {
					let _permit = budget.acquire("wide", 2).unwrap();
				});
			}
		});
		let _all = budget.acquire("all", 2).unwrap();
		assert_eq!(budget.client.available().unwrap(), 0);
	}
	#[test]
	fn native_uses_free_capacity_without_waiting_for_interpreters() {
		let budget = Budget::new(3).unwrap();
		let interpreter = budget.acquire("interpreter", 1).unwrap();
		let native = budget.acquire_up_to("native", 3).unwrap();
		assert_eq!(native.slots(), 2);
		assert_eq!(budget.client.available().unwrap(), 0);
		drop(native);
		assert_eq!(budget.client.available().unwrap(), 2);
		drop(interpreter);
		let all = budget.acquire_up_to("native-alone", 3).unwrap();
		assert_eq!(all.slots(), 3);
	}

	#[test]
	fn child_shares_the_parent_jobserver() {
		let budget = Budget::new(2).unwrap();
		let _implicit = budget.acquire("cargo", 1).unwrap();
		let mut command = Command::new(std::env::current_exe().unwrap());
		command
			.args([
				"--exact",
				"scheduler::tests::jobserver_child",
				"--ignored",
				"--nocapture",
				"--test-threads=1",
			])
			.env("V4_JOBSERVER_CHILD", "1");
		budget.configure(&mut command);
		let output = command.output().unwrap();
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
		assert!(String::from_utf8(output.stdout)
			.unwrap()
			.contains("SHARED_JOBSERVER_OK"));
	}
	#[test]
	#[ignore = "internal process-level jobserver probe"]
	fn jobserver_child() {
		assert_eq!(std::env::var("V4_JOBSERVER_CHILD").unwrap(), "1");
		// Only the descriptor set explicitly inherited by this test's parent is read.
		let client = unsafe { jobserver::Client::from_env() }.expect("shared jobserver");
		let _extra = client.acquire().unwrap();
		assert_eq!(client.available().unwrap(), 0);
		println!("SHARED_JOBSERVER_OK");
		if std::env::var_os("V4_EXIT_WITH_TOKEN").is_some() {
			std::process::exit(7);
		}
	}
	#[test]
	fn canceled_build_wakes_blocked_launchers_without_inventing_tokens() {
		let budget = Budget::new(1).unwrap();
		let permit = budget.acquire("running", 1).unwrap();
		let ready = std::sync::Barrier::new(2);
		std::thread::scope(|scope| {
			let child = scope.spawn(|| {
				ready.wait();
				assert!(budget.acquire("blocked", 1).is_err());
			});
			ready.wait();
			budget.abort();
			child.join().unwrap();
		});
		assert!(budget.acquire("new", 1).is_err());
		assert_eq!(budget.client.available().unwrap(), 0);
		drop(permit);
	}
	#[test]
	fn wide_waiter_does_not_hoard_partial_pipe_tokens() {
		let budget = Budget::new(3).unwrap();
		let running = budget.acquire("interpreter", 1).unwrap();
		std::thread::scope(|scope| {
			let wide = scope.spawn(|| {
				let _all = budget.acquire("native", 3).unwrap();
			});
			let mut state = budget.admission.lock().unwrap();
			while state.wide_waiters == 0 {
				state = budget.released.wait(state).unwrap();
			}
			assert_eq!(state.reserved, 1);
			assert_eq!(budget.client.available().unwrap(), 2);
			drop(state);
			// A whole-machine waiter must not block ready independent work.
			let ready = budget.acquire("ready-narrow", 1).unwrap();
			drop(ready);
			drop(running);
			wide.join().unwrap();
		});
	}
	#[test]
	fn older_fitting_request_cannot_be_starved_by_new_work() {
		let budget = Budget::new(1).unwrap();
		let held = budget.acquire("existing", 1).unwrap();
		let order = Mutex::new(Vec::new());
		std::thread::scope(|scope| {
			let first = scope.spawn(|| {
				let _permit = budget.acquire("older", 1).unwrap();
				order.lock().unwrap().push("older");
			});
			let mut state = budget.admission.lock().unwrap();
			while state.waiting.is_empty() {
				state = budget.released.wait(state).unwrap();
			}
			drop(state);
			drop(held);
			{
				let _permit = budget.acquire("newer", 1).unwrap();
				order.lock().unwrap().push("newer");
			}
			first.join().unwrap();
		});
		assert_eq!(*order.lock().unwrap(), vec!["older", "newer"]);
	}
}
