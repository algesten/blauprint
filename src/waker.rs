use std::ptr;
use std::task::{RawWaker, RawWakerVTable, Waker};

pub fn fake_waker() -> Waker {
    let waker = RawWaker::new(ptr::null(), &RAW_WAKER_VTABLE);
    unsafe { Waker::from_raw(waker) }
}

const RAW_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

fn clone(_ptr: *const ()) -> RawWaker {
    panic!("Blueprint is not an async runtime.");
}

fn wake(_ptr: *const ()) {}

fn wake_by_ref(_ptr: *const ()) {}

fn drop(_ptr: *const ()) {}
