use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use waker::fake_waker;

mod waker;

pub fn run<Input, Output, Fut>(
    blueprint: impl FnOnce(Handle<Input, Output>) -> Fut,
) -> impl Iterator<Item = Event<Input, Output, Fut::Output>>
where
    Input: 'static,
    Output: 'static,
    Fut: Future + 'static,
    Fut::Output: 'static,
{
    let inner = SharedCell::new();

    let handle = Handle {
        inner: inner.clone(),
    };

    let future = blueprint(handle);

    Executor::new(future, inner)
}

pub struct Handle<Input, Output> {
    inner: SharedCell<InnerEvent<Input, Output>>,
}

impl<Input, Output> Handle<Input, Output> {
    pub fn want_input(&self) -> impl Future<Output = Input> {
        let holder = SharedCell::new();
        let responder = Responder::new(holder.clone());

        let event = InnerEvent::Input(responder);
        self.inner.0.set(Some(event));

        WantInput::new(holder)
    }

    pub fn provide_output(&self, output: Output) -> impl Future<Output = ()> {
        let event = InnerEvent::Output(output);
        self.inner.0.set(Some(event));
        Pause::default()
    }
}

#[derive(Default)]
struct Pause(bool);

impl Future for Pause {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

struct WantInput<T> {
    holder: SharedCell<T>,
}
impl<T> WantInput<T> {
    fn new(holder: SharedCell<T>) -> Self {
        Self { holder }
    }
}

impl<T> Future for WantInput<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.holder.take() {
            Some(v) => Poll::Ready(v),
            None => Poll::Pending,
        }
    }
}

struct Executor<Input, Output, Fut> {
    future: Option<Pin<Box<Fut>>>,
    inner: SharedCell<InnerEvent<Input, Output>>,
    waker: Waker,
}
impl<Input, Output, Fut: Future> Executor<Input, Output, Fut> {
    fn new(future: Fut, inner: SharedCell<InnerEvent<Input, Output>>) -> Self {
        Self {
            future: Some(Box::pin(future)),
            inner,
            waker: fake_waker(),
        }
    }
}

impl<I: 'static, O: 'static, Fut: Future> Iterator for Executor<I, O, Fut> {
    type Item = Event<I, O, Fut::Output>;

    fn next(&mut self) -> Option<Self::Item> {
        let future = self.future.as_mut()?;
        let mut cx = Context::from_waker(&self.waker);

        match Pin::new(future).poll(&mut cx) {
            Poll::Ready(v) => {
                // Any more calls to next() will yield None.
                self.future = None;
                Some(Event::Return(v))
            }
            Poll::Pending => {
                let inner_event = self.inner.0.replace(None).expect("inner event");
                Some(inner_event.into())
            }
        }
    }
}

pub enum Event<Input, Output, Return> {
    Input(Responder<Input>),
    Output(Output),
    Return(Return),
}

enum InnerEvent<Input, Output> {
    Input(Responder<Input>),
    Output(Output),
}

impl<Input, Output, Return> Into<Event<Input, Output, Return>> for InnerEvent<Input, Output> {
    fn into(self) -> Event<Input, Output, Return> {
        match self {
            InnerEvent::Input(i) => Event::Input(i),
            InnerEvent::Output(o) => Event::Output(o),
        }
    }
}

pub struct Responder<T = ()> {
    holder: SharedCell<T>,
}

impl<T> Responder<T> {
    fn new(holder: SharedCell<T>) -> Self {
        Self { holder }
    }

    pub fn provide(self, data: T) {
        self.holder.set(data);
    }
}

impl Responder<()> {
    pub fn resume(self) {
        self.provide(());
    }
}

struct SharedCell<T>(Rc<Cell<Option<T>>>);

impl<T> SharedCell<T> {
    fn new() -> Self {
        SharedCell(Rc::new(Cell::new(None)))
    }

    fn set(&self, value: T) {
        let x = self.0.replace(Some(value));
        assert!(x.is_none(), "SharedCell was already set");
    }

    fn take(&self) -> Option<T> {
        self.0.replace(None)
    }

    fn clone(&self) -> Self {
        SharedCell(self.0.clone())
    }
}

#[cfg(test)]
mod test {
    use std::marker::PhantomData;

    use super::*;

    #[allow(non_camel_case_types)]
    struct NEED_DATA;
    struct FINISHED;

    #[derive(Default)]
    struct MyThing<T> {
        data: Option<u8>,
        _ph: PhantomData<T>,
    }

    impl MyThing<()> {
        fn new() -> MyThing<NEED_DATA> {
            MyThing {
                data: None,
                _ph: PhantomData,
            }
        }
    }

    impl MyThing<NEED_DATA> {
        fn gief_input(&mut self, input: u8) {
            self.data = Some(input);
        }

        fn proceed(self) -> Option<MyThing<FINISHED>> {
            let data = self.data?;
            Some(MyThing {
                data: Some(data),
                _ph: PhantomData,
            })
        }
    }

    impl MyThing<FINISHED> {
        pub fn get_data(&self) -> u8 {
            // unwrap is ok because state guarantees it.
            self.data.unwrap()
        }
    }

    async fn test_blauprint(handle: Handle<u8, String>) -> &'static str {
        // This is a Thing<NEED_DATA>
        let mut thing = MyThing::new();

        let input = handle.want_input().await;

        // gief_input() only implemented for MyThing<NEED_DATA>
        thing.gief_input(input);

        // Rebind to Thing<FINISHED>
        let thing = thing.proceed().expect("thing to be able to proceed");

        // get_data() only implemented for MyThing<FINISHED>
        let output = thing.get_data().to_string();

        handle.provide_output(output).await;

        "alles gut"
    }

    #[test]
    fn test() {
        let events = run(test_blauprint);

        for io in events {
            match io {
                Event::Input(res) => {
                    println!("provide input");
                    res.provide(42);
                }
                Event::Output(out) => {
                    println!("output: {}", out);
                    assert_eq!(out, "42");
                }
                Event::Return(end) => {
                    println!("end");
                    assert_eq!(end, "alles gut");
                }
            }
        }
    }
}
