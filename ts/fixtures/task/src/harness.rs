//! Runs a script of executor steps and returns the log: the native side of the
//! differential test, which `tests/task.ts` mirrors step for step on the translation.
//!
//! Each step is two bytes, an op and its argument, and logs its name, so the two sides
//! differ wherever their lists do. Channels and tasks are picked by the argument, modulo
//! how many there are; a step with nothing to pick does nothing. Once the script ends,
//! every task drops in order, then every channel.

use crate::drops;
use crate::exec::Executor;
use crate::futures;
use crate::log::Log;
use crate::model::{self, Channel, Guard};

/// Run one script.
pub fn run(input: &[u8]) -> Vec<u8> {
	let log = Log::new();
	let mut exec = Executor::new(log.clone());
	let mut channels: Vec<Channel> = Vec::new();
	let steps = steps();
	for step in input.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		let (name, run_step) = steps[usize::from(op) % steps.len()];
		log.note(format!("step {name}"));
		run_step(&log, &mut exec, &mut channels, arg);
	}
	for t in 0..exec.len() {
		exec.drop(t);
	}
	for (c, channel) in channels.into_iter().enumerate() {
		log.note(format!("c{c} drop"));
		drop(channel);
	}
	log.text()
}

/// What a step does with its argument.
type Step = fn(&Log, &mut Executor, &mut Vec<Channel>, u8);

/// Each step and its name, which a script's op picks modulo their count: `tests/task.ts`'s
/// `steps`, in the same order. A step added at the end moves no other.
fn steps() -> Vec<(&'static str, Step)> {
	vec![
		("open", |log, _, channels, _| {
			log.note(format!("c{} open", channels.len()));
			channels.push(Channel::open());
		}),
		("sum", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(model::sum(channels[c].consumer(), guard));
			}
		}),
		("drain", |_, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let depth = [1, 4, 16][usize::from(arg / 16) % 3];
				exec.spawn(model::drain(channels[c].consumer(), depth));
			}
		}),
		("send", |log, _, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				log.note(format!("c{c} send {arg}"));
				if !channels[c].send(u64::from(arg)) {
					log.note(format!("c{c} closed"));
				}
			}
		}),
		("close", |log, _, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				log.note(format!("c{c} close"));
				channels[c].close();
			}
		}),
		("clone", |log, _, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				log.note(format!("c{c} clone"));
				channels[c].clone_producer();
			}
		}),
		("run", |_, exec, _, _| exec.run()),
		("step", |_, exec, _, _| _ = exec.step()),
		("poll", |_, exec, _, arg| {
			if let Some(t) = pick(exec.len(), arg) {
				exec.poll(t);
			}
		}),
		("drop", |_, exec, _, arg| {
			if let Some(t) = pick(exec.len(), arg) {
				exec.drop(t);
			}
		}),
		("blocks", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(model::blocks(channels[c].consumer(), guard, arg >= 128));
			}
		}),
		("drops", |log, exec, _, arg| {
			drops::run(log, 1000 * exec.len() as u32, arg)
		}),
		("held", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(drops::held(channels[c].consumer(), guard));
			}
		}),
		("edited", |log, exec, _, arg| {
			drops::edited(log, 1000 * exec.len() as u32, arg)
		}),
		("helpers", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(futures::helpers(channels[c].consumer(), guard));
			}
		}),
		("boxed", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(futures::boxed(channels[c].consumer(), guard));
			}
		}),
		("dyns", |log, exec, _, arg| {
			drops::dyns(log, 1000 * exec.len() as u32, arg)
		}),
		("cells", |log, exec, _, arg| {
			drops::cells(log, 1000 * exec.len() as u32, arg)
		}),
		("folded", |log, exec, _, arg| {
			drops::folded(log, 1000 * exec.len() as u32, arg)
		}),
		("rfolded", |log, exec, _, arg| {
			drops::rfolded(log, 1000 * exec.len() as u32, arg)
		}),
		("watch", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(model::watch(channels[c].consumer(), guard));
			}
		}),
		("front", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(model::front(channels[c].consumer(), guard));
			}
		}),
		("inside", |log, exec, _, arg| {
			drops::inside(log, 1000 * exec.len() as u32, arg)
		}),
		("compared", |log, exec, _, arg| {
			drops::compared(log, 1000 * exec.len() as u32, arg)
		}),
		("wakers", |log, exec, channels, arg| {
			if let Some(c) = pick(channels.len(), arg) {
				let guard = Guard::new(exec.len() as u32, log.clone());
				exec.spawn(futures::wakers(channels[c].consumer(), guard));
			}
		}),
		("lists", |log, exec, _, arg| {
			drops::lists(log, 1000 * exec.len() as u32, arg)
		}),
	]
}

/// The one of `len` channels or tasks that `arg` picks, if there are any.
fn pick(len: usize, arg: u8) -> Option<usize> {
	(len > 0).then(|| usize::from(arg) % len)
}
