use std::collections::HashMap;
use std::future::Future;
use std::hash::{Hasher, Hash};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Instant, Duration};

impl Hash for Point {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.latitude.hash(state);
        self.longitude.hash(state);
    }
}

use tokio::select;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use tonic::{metadata::MetadataValue, transport::server::RoutesBuilder, transport::Server, Request, Response, Status};
use tonic_health::server::HealthReporter;

use hello_world::{HelloReply, HelloRequest, greeter_server::{Greeter, GreeterServer}};

pub mod hello_world {
    tonic::include_proto!("helloworld"); // The string specified here must match the proto package name
}

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    #[tracing::instrument]
    async fn say_hello(
        &self,
        request: Request<HelloRequest>, // Accept request of type HelloRequest
    ) -> Result<Response<HelloReply>, Status> { // Return an instance of type HelloReply
        let remote_addr = request.remote_addr();
        println!("Got a request: {:?}", request);
        let request_future = async move {
            println!("Got a request from {:?}", request.remote_addr());

            let extension = request.extensions().get::<Extension>().unwrap();
            println!("extension data = {}", extension.some_piece_of_data);

            let reply = hello_world::HelloReply {
                message: format!("Hello {}!", request.into_inner().name), // We must use .into_inner() as the fields of gRPC requests and responses are private
            };

            Ok(Response::new(reply)) // Send back our formatted greeting
        };
        let cancellation_future = async move {
            println!("Request from {:?} cancelled by client", remote_addr);
            // If this future is executed it means the request future was dropped,
            // so it doesn't actually matter what is returned here
            Err(Status::cancelled("Request cancelled by client"))
        };
        with_cancellation_handler(request_future, cancellation_future).await
    }
}

async fn with_cancellation_handler<FRequest, FCancellation>(
    request_future: FRequest,
    cancellation_future: FCancellation,
) -> Result<Response<HelloReply>, Status>
where
    FRequest: Future<Output = Result<Response<HelloReply>, Status>> + Send + 'static,
    FCancellation: Future<Output = Result<Response<HelloReply>, Status>> + Send + 'static,
{
    let token = CancellationToken::new();
    // Will call token.cancel() when the future is dropped, such as when the client cancels the request
    let _drop_guard = token.clone().drop_guard();
    let select_task = tokio::spawn(async move {
        // Can select on token cancellation on any cancellable future while handling the request,
        // allowing for custom cleanup code or monitoring
        select! {
            res = request_future => res,
            _ = token.cancelled() => cancellation_future.await,
        }
    });

    select_task.await.unwrap()
}

use route_guide::{Feature, Point, Rectangle, RouteNote, RouteSummary, route_guide_server::{RouteGuide, RouteGuideServer}};

pub mod route_guide {
    tonic::include_proto!("routeguide");
}

#[derive(Debug)]
pub struct RouteGuideService {
    features: Arc<Vec<Feature>>,
}

#[tonic::async_trait]
impl RouteGuide for RouteGuideService {
    async fn get_feature(&self, _request: Request<Point>) -> Result<Response<Feature>, Status> {
        for feature in &self.features[..] {
            if feature.location.as_ref() == Some(_request.get_ref()) {
                return Ok(Response::new(feature.clone()));
            }
        }
    
        Ok(Response::new(Feature::default()))
    }

    type ListFeaturesStream = ReceiverStream<Result<Feature, Status>>;

    async fn list_features(
        &self,
        _request: Request<Rectangle>,
    ) -> Result<Response<Self::ListFeaturesStream>, Status> {
        let (tx, rx) = mpsc::channel(4);
        let features = self.features.clone();
    
        tokio::spawn(async move {
            for feature in &features[..] {
                if in_range(feature.location.as_ref().unwrap(), _request.get_ref()) {
                    tx.send(Ok(feature.clone())).await.unwrap();
                }
            }
        });
    
        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn record_route(
        &self,
        _request: Request<tonic::Streaming<Point>>,
    ) -> Result<Response<RouteSummary>, Status> {
        let mut stream = _request.into_inner();

        let mut summary = RouteSummary::default();
        let mut last_point = None;
        let now = Instant::now();
    
        while let Some(point) = stream.next().await {
            let point = point?;
            summary.point_count += 1;
    
            for feature in &self.features[..] {
                if feature.location.as_ref() == Some(&point) {
                    summary.feature_count += 1;
                }
            }
    
            if let Some(ref last_point) = last_point {
                summary.distance += calc_distance(last_point, &point);
            }
    
            last_point = Some(point);
        }
    
        summary.elapsed_time = now.elapsed().as_secs() as i32;
    
        Ok(Response::new(summary))
    }

    type RouteChatStream = Pin<Box<dyn Stream<Item = Result<RouteNote, Status>> + Send  + 'static>>;

    async fn route_chat(
        &self,
        _request: Request<tonic::Streaming<RouteNote>>,
    ) -> Result<Response<Self::RouteChatStream>, Status> {
        let mut notes = HashMap::new();
        let mut stream = _request.into_inner();
    
        let output = async_stream::try_stream! {
            while let Some(note) = stream.next().await {
                let note = note?;
    
                let location = note.location.clone().unwrap();
    
                let location_notes = notes.entry(location).or_insert(vec![]);
                location_notes.push(note);
    
                for note in location_notes {
                    yield note.clone();
                }
            }
        };
    
        Ok(Response::new(Box::pin(output)
            as Self::RouteChatStream))
    }
}

/// This function (somewhat improbably) flips the status of a service every second, in order
/// that the effect of `tonic_health::HealthReporter::watch` can be easily observed.
async fn twiddle_service_status(mut reporter: HealthReporter) {
    let mut iter = 0u64;
    loop {
        iter += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;

        if iter % 2 == 0 {
            reporter.set_serving::<GreeterServer<MyGreeter>>().await;
        } else {
            reporter.set_not_serving::<GreeterServer<MyGreeter>>().await;
        };
    }
}

async fn init_greeter(builder: &mut RoutesBuilder) {
    println!("Adding Greeter service...");
    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<GreeterServer<MyGreeter>>()
        .await;
    tokio::spawn(twiddle_service_status(health_reporter.clone()));
    let greeter = MyGreeter::default();
    builder.add_service(health_service);
    builder.add_service(GreeterServer::with_interceptor(greeter, intercept));
}

fn init_route_guide(builder: &mut RoutesBuilder) {
    println!("Adding Route Guide service...");
    let route_guide = RouteGuideService {
        features: Arc::new(data::load()),
    };
    let route_guide_server = RouteGuideServer::with_interceptor(route_guide, check_auth);
    builder.add_service(route_guide_server);
}

mod data;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let addr = "[::1]:50051".parse().unwrap();
    let mut routes_builder = RoutesBuilder::default();
    init_greeter(&mut routes_builder).await;
    init_route_guide(&mut routes_builder);
    
    Server::builder()
        .trace_fn(|_| tracing::info_span!("helloworld_server"))
        .accept_http1(true)
        .add_routes(routes_builder.routes())
        .serve(addr)
        .await?;

    Ok(())
}

impl Eq for Point {}
fn in_range(point: &Point, rect: &Rectangle) -> bool {
    use std::cmp;

    let lo = rect.lo.as_ref().unwrap();
    let hi = rect.hi.as_ref().unwrap();

    let left = cmp::min(lo.longitude, hi.longitude);
    let right = cmp::max(lo.longitude, hi.longitude);
    let top = cmp::max(lo.latitude, hi.latitude);
    let bottom = cmp::min(lo.latitude, hi.latitude);

    point.longitude >= left
        && point.longitude <= right
        && point.latitude >= bottom
        && point.latitude <= top
}

/// Calculates the distance between two points using the "haversine" formula.
/// This code was taken from http://www.movable-type.co.uk/scripts/latlong.html.
fn calc_distance(p1: &Point, p2: &Point) -> i32 {
    const CORD_FACTOR: f64 = 1e7;
    const R: f64 = 6_371_000.0; // meters

    let lat1 = p1.latitude as f64 / CORD_FACTOR;
    let lat2 = p2.latitude as f64 / CORD_FACTOR;
    let lng1 = p1.longitude as f64 / CORD_FACTOR;
    let lng2 = p2.longitude as f64 / CORD_FACTOR;

    let lat_rad1 = lat1.to_radians();
    let lat_rad2 = lat2.to_radians();

    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lng = (lng2 - lng1).to_radians();

    let a = (delta_lat / 2f64).sin() * (delta_lat / 2f64).sin()
        + (lat_rad1).cos() * (lat_rad2).cos() * (delta_lng / 2f64).sin() * (delta_lng / 2f64).sin();

    let c = 2f64 * a.sqrt().atan2((1f64 - a).sqrt());

    (R * c) as i32
}

/// This function will get called on each inbound request, if a `Status`
/// is returned, it will cancel the request and return that status to the
/// client.
fn intercept(mut req: Request<()>) -> Result<Request<()>, Status> {
    println!("Intercepting request: {:?}", req);

    // Set an extension that can be retrieved by `say_hello`
    req.extensions_mut().insert(Extension {
        some_piece_of_data: "foo".to_string(),
    });

    Ok(req)
}

struct Extension {
    some_piece_of_data: String,
}

fn check_auth(req: Request<()>) -> Result<Request<()>, Status> {
    let token: MetadataValue<_> = "Bearer some-auth-token".parse().unwrap();
    println!("Check auth: {:?}", req.metadata().get("authorization"));
    match req.metadata().get("authorization") {
        Some(t) if token == t => Ok(req),
        _ => Err(Status::unauthenticated("No valid auth token")),
    }
}