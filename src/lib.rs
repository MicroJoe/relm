/*
 * Copyright (c) 2017 Boucher, Antoni <bouanto@zoho.com>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of
 * this software and associated documentation files (the "Software"), to deal in
 * the Software without restriction, including without limitation the rights to
 * use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
 * the Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
 * FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
 * COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
 * IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
 * CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */

/*
 * TODO: convert GTK+ callback to Stream.
 * TODO: communication across widgets.
 * TODO: chat client/server example.
 *
 * TODO: try tk-easyloop in another branch.
 *
 * TODO: add Cargo travis badge.
 * TODO: use macros 2.0 instead for the:
 * * view: to create the dependencies between the view items and the model.
 * * model: to add boolean fields in an inner struct specifying which parts of the view to update
 * *        after the update.
 * * update: to set the boolean fields to true depending on which parts of the model was updated.
 * * create default values for gtk widgets (like Label::new(None)).
 * * create attributes for constructor gtk widgets (like orientation for Box::new(orientation)).
 * TODO: optionnaly multi-threaded.
 * TODO: Use two update functions (one for errors, one for success/normal behavior).
 */

extern crate futures;
extern crate gtk;
extern crate relm_core;

mod macros;
mod widget;

use std::error;
use std::fmt::{self, Display, Formatter};
use std::io;

use futures::{Future, IntoStream, Poll, Stream};
use gtk::ContainerExt;
use relm_core::Core;
pub use relm_core::{EventStream, Handle, QuitFuture};

pub use self::Error::*;
pub use self::widget::*;

pub type UnitFuture = Box<Future<Item=(), Error=()>>;
pub type UnitStream = Box<Stream<Item=(), Error=()>>;

pub struct RelmStream<E, I, S: Stream<Item=I, Error=E>> {
    stream: S,
}

impl<E, I, S: Stream<Item=I, Error=E>> Stream for RelmStream<E, I, S> {
    type Item = I;
    type Error = E;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.stream.poll()
    }
}

pub trait ToStream<S: Stream<Item=Self::Item, Error=Self::Error>> {
    type Error;
    type Item;

    fn to_stream(self) -> RelmStream<Self::Error, Self::Item, S>;
}

impl<E, I, S: Stream<Item=I, Error=E>> ToStream<S> for S {
    type Error = E;
    type Item = I;

    fn to_stream(self) -> RelmStream<E, I, S> {
        RelmStream {
            stream: self,
        }
    }
}

impl<E, F: Future<Item=I, Error=E>, I> ToStream<IntoStream<F>> for F {
    type Error = E;
    type Item = I;

    fn to_stream(self) -> RelmStream<E, I, IntoStream<F>> {
        RelmStream {
            stream: self.into_stream(),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    GtkInit,
    Io(io::Error),
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        match *self {
            GtkInit => write!(formatter, "Cannot init GTK+"),
            Io(ref error) => write!(formatter, "IO error: {}", error),
        }
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            GtkInit => "Cannot init GTK+",
            Io(ref error) => error.description(),
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            GtkInit => None,
            Io(ref error) => Some(error),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Error {
        Io(error)
    }
}

impl From<()> for Error {
    fn from((): ()) -> Error {
        GtkInit
    }
}

pub struct Relm<M> {
    handle: Handle,
    stream: EventStream<M>,
}

impl<M: Clone + 'static> Relm<M> {
    pub fn connect<C, S, T>(&self, to_stream: T, callback: C) -> UnitFuture
        where C: Fn(S::Item) -> M + 'static,
              S: Stream + 'static,
              T: ToStream<S, Item=S::Item, Error=S::Error> + 'static,
    {
        let event_stream = self.stream.clone();
        let stream = to_stream.to_stream();
        Box::new(stream.and_then(move |result| {
            event_stream.emit(callback(result));
            Ok(())
        })
            .for_each(Ok)
            // TODO: handle errors.
            .map_err(|_| ()))
    }

    pub fn exec<F: Future<Item=(), Error=()> + 'static>(&self, future: F) {
        self.handle.spawn(future);
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn run<D: Widget<M> + 'static>() -> Result<(), Error> {
        gtk::init()?;

        let mut core = Core::new()?;

        let handle = core.handle();
        create_widget::<D, M>(&handle);

        core.run();
        Ok(())
    }

    // TODO: delete this method when the connect macros are no longer required.
    pub fn stream(&self) -> &EventStream<M> {
        &self.stream
    }
}

fn create_widget<D: Widget<M> + 'static, M: Clone + 'static>(handle: &Handle) -> D::Container {
    let stream = EventStream::new();

    let relm = Relm {
        handle: handle.clone(),
        stream: stream.clone(),
    };
    let mut widget = D::new(relm);
    widget.connect_events();

    let subscriptions = widget.subscriptions();
    for subscription in subscriptions {
        handle.spawn(subscription);
    }

    let container = widget.container().clone();

    let event_future = {
        let handle = handle.clone();
        stream.for_each(move |event| {
            let future = widget.update(event);
            handle.spawn(future);
            Ok(())
        })
    };
    handle.spawn(event_future);

    container
}

pub trait AddWidget
    where Self: ContainerExt
{
    fn add_widget<D: Widget<M> + 'static, M: Clone + 'static>(&self, handle: &Handle) {
        let widget = create_widget::<D, M>(handle);
        self.add(&widget);
    }
}

impl<W: ContainerExt> AddWidget for W { }