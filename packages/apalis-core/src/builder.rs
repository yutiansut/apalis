use std::{error::Error, fmt::Debug, marker::PhantomData};

use futures::Stream;
use tower::{
    layer::util::{Identity, Stack},
    Layer, Service, ServiceBuilder,
};

use crate::{
    job::Job,
    job_fn::{job_fn, JobFn},
    request::JobRequest,
    worker::{ready::ReadyWorker, Worker, WorkerRef},
};

/// An abstract that allows building a [`Worker`].
/// Usually the output is [`ReadyWorker`] but you can implement your own via [`WorkerFactory`]
#[derive(Debug)]
pub struct WorkerBuilder<Job, Source, Middleware> {
    pub(crate) name: String,
    pub(crate) job: PhantomData<Job>,
    pub(crate) layer: ServiceBuilder<Middleware>,
    pub(crate) source: Source,
}

impl WorkerBuilder<(), (), Identity> {
    /// Build a new [`WorkerBuilder`] instance with a name for the worker to build
    pub fn new<N: Into<String>>(name: N) -> WorkerBuilder<(), (), Identity> {
        let job: PhantomData<()> = PhantomData;
        WorkerBuilder {
            job,
            layer: ServiceBuilder::new(),
            source: (),
            name: name.into(),
        }
    }
}

impl<J, S, M> WorkerBuilder<J, S, M> {
    /// Consume a stream directly
    pub fn stream<NS: Stream<Item = Result<Option<JobRequest<NJ>>, E>>, E, NJ>(
        self,
        stream: NS,
    ) -> WorkerBuilder<NJ, NS, M> {
        WorkerBuilder {
            job: PhantomData,
            layer: self.layer,
            source: stream,
            name: self.name,
        }
    }

    /// Get the [`WorkerRef`] and build a stream.
    /// Useful when you want to know what worker is consuming the stream.
    pub fn with_stream<
        NS: Fn(WorkerRef) -> ST,
        NJ,
        E,
        ST: Stream<Item = Result<Option<JobRequest<NJ>>, E>>,
    >(
        self,
        stream: NS,
    ) -> WorkerBuilder<NJ, ST, M> {
        WorkerBuilder {
            job: PhantomData,
            layer: self.layer,
            source: stream(WorkerRef::new(self.name.clone())),
            name: self.name,
        }
    }
}

impl<Job, Stream, Serv> WorkerBuilder<Job, Stream, Serv> {
    /// Allows of decorating the service that consumes jobs.
    /// Allows adding multiple [`tower`] middleware
    pub fn middleware<NewService>(
        self,
        f: impl Fn(ServiceBuilder<Serv>) -> ServiceBuilder<NewService>,
    ) -> WorkerBuilder<Job, Stream, NewService> {
        let middleware = f(self.layer);

        WorkerBuilder {
            job: self.job,
            layer: middleware,
            name: self.name,
            source: self.source,
        }
    }
    /// Shorthand for decoration. Allows adding a single layer [tower] middleware
    pub fn layer<U>(self, layer: U) -> WorkerBuilder<Job, Stream, Stack<U, Serv>>
    where
        Serv: Layer<U>,
    {
        WorkerBuilder {
            job: self.job,
            source: self.source,
            layer: self.layer.layer(layer),
            name: self.name,
        }
    }
}

impl<J, S, M, Ser, E, Request> WorkerFactory<J, Ser> for WorkerBuilder<J, S, M>
where
    S: Stream<Item = Result<Option<Request>, E>> + Send + 'static + Unpin,
    J: Job + Send + 'static,
    M: Layer<Ser>,
    <M as Layer<Ser>>::Service: Service<Request> + Send + 'static,
    E: Sync + Send + 'static + Error,
    Request: Send,
    <<M as Layer<Ser>>::Service as Service<Request>>::Future: std::marker::Send,
    Ser: Service<Request>,
    <Ser as Service<Request>>::Error: Debug,
    <<M as Layer<Ser>>::Service as Service<Request>>::Error: std::fmt::Debug,
    <<M as Layer<Ser>>::Service as Service<Request>>::Future: 'static,
{
    type Worker = ReadyWorker<S, <M as Layer<Ser>>::Service>;
    /// Convert a worker builder to a worker ready to consume jobs
    fn build(self, service: Ser) -> ReadyWorker<S, <M as Layer<Ser>>::Service> {
        ReadyWorker {
            name: self.name,
            stream: self.source,
            service: self.layer.service(service),
        }
    }
}

/// Helper trait for building new Workers from [`WorkerBuilder`]
pub trait WorkerFactory<J, S> {
    /// The worker to build
    type Worker: Worker<J>;
    /// Builds a [`WorkerFactory`] using a [`tower`] service
    /// that can be used to generate new [`Worker`] actors using the `build` method
    /// # Arguments
    ///
    /// * `service` - A tower service
    ///
    /// # Examples
    ///
    fn build(self, service: S) -> Self::Worker;
}

/// Helper trait for building new Workers from [`WorkerBuilder`]

pub trait WorkerFactoryFn<J, F> {
    /// The worker build
    type Worker: Worker<J>;
    /// Builds a [`WorkerFactoryFn`] using a [`crate::job_fn::JobFn`] service
    /// that can be used to generate new [`Worker`] actors using the `build` method
    /// # Arguments
    ///
    /// * `f` - A tower functional service
    ///
    /// # Examples
    ///
    fn build_fn(self, f: F) -> Self::Worker;
}

impl<J, W, F> WorkerFactoryFn<J, F> for W
where
    W: WorkerFactory<J, JobFn<F>>,
{
    type Worker = W::Worker;

    fn build_fn(self, f: F) -> Self::Worker {
        self.build(job_fn(f))
    }
}
