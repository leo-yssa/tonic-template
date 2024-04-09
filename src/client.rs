use std::error::Error;

use rand::rngs::ThreadRng;
use rand::Rng;

use tokio::time::{Duration, Instant, interval, timeout};

use tonic::{
    codegen::InterceptedService,
    metadata::MetadataValue,
    service::Interceptor,
    transport::{Channel, Endpoint},
    Request, Status,
};
use tonic_web::GrpcWebClientLayer;

use hello_world::{HelloRequest, greeter_client::GreeterClient};

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

use route_guide::{Point, Rectangle, RouteNote, route_guide_client::RouteGuideClient};

pub mod route_guide {
    tonic::include_proto!("routeguide");
}

async fn print_features(client: &mut RouteGuideClient<InterceptedService<Channel, impl Fn(Request<()>) -> Result<Request<()>, Status>>>) -> Result<(), Box<dyn Error>> {
    let rectangle = Rectangle {
        lo: Some(Point {
            latitude: 400_000_000,
            longitude: -750_000_000,
        }),
        hi: Some(Point {
            latitude: 420_000_000,
            longitude: -730_000_000,
        }),
    };

    let mut stream = client
        .list_features(Request::new(rectangle))
        .await?
        .into_inner();

    while let Some(feature) = stream.message().await? {
        println!("NOTE = {:?}", feature);
    }

    Ok(())
}

async fn run_record_route(client: &mut RouteGuideClient<InterceptedService<Channel, impl Fn(Request<()>) -> Result<Request<()>, Status>>>) -> Result<(), Box<dyn Error>> {
    let mut rng = rand::thread_rng();
    let point_count: i32 = rng.gen_range(2..100);

    let mut points = vec![];
    for _ in 0..=point_count {
        points.push(random_point(&mut rng))
    }

    println!("Traversing {} points", points.len());
    let request = Request::new(tokio_stream::iter(points));

    match client.record_route(request).await {
        Ok(response) => println!("SUMMARY: {:?}", response.into_inner()),
        Err(e) => println!("something went wrong: {:?}", e),
    }

    Ok(())
}

async fn run_route_chat(client: &mut RouteGuideClient<InterceptedService<Channel, impl Fn(Request<()>) -> Result<Request<()>, Status>>>) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();

    let outbound = async_stream::stream! {
        let mut interval = interval(Duration::from_secs(1));

        loop {
            let time = interval.tick().await;
            let elapsed = time.duration_since(start);
            let note = RouteNote {
                location: Some(Point {
                    latitude: 409146138 + elapsed.as_secs() as i32,
                    longitude: -746188906,
                }),
                message: format!("at {:?}", elapsed),
            };

            yield note;
        }
    };

    let response = client.route_chat(Request::new(outbound)).await?;
    let mut inbound = response.into_inner();

    while let Some(note) = inbound.message().await? {
        println!("NOTE = {:?}", note);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    let channel = Endpoint::from_static("http://[::1]:50051")
        .connect()
        .await?;
    let mut hello_world_client = GreeterClient::with_interceptor(channel.clone(), intercept);

    let hello_request = Request::new(HelloRequest {
        name: "Tonic".into(),
    });
    tracing::info!(
        message = "Sending request.",
        request = %hello_request.get_ref().name
    );
    let hello_response = match timeout(Duration::from_secs(1), hello_world_client.say_hello(hello_request)).await {
        Ok(response) => response?,
        Err(_) => {
            println!("Cancelled request after 1s");
            return Ok(());
        }
    };

    tracing::info!(
        message = "Got a response.",
        response = %hello_response.get_ref().message
    );

    let hyper_client = hyper::Client::builder().build_http();
    let svc = tower::ServiceBuilder::new()
        .layer(GrpcWebClientLayer::new())
        .service(hyper_client);
    let mut greeter_client_with_hyper = GreeterClient::with_origin(svc, "http://[::1]:50051".try_into()?);

    let request = tonic::Request::new(HelloRequest {
        name: "Tonic".into(),
    });

    let response = greeter_client_with_hyper.say_hello(request).await?;

    println!("RESPONSE={:?}", response);

    let token: MetadataValue<_> = "Bearer some-auth-token".parse()?;
    let mut route_guide_client = RouteGuideClient::with_interceptor(channel.clone(), move |mut req: Request<()>| {
        req.metadata_mut().insert("authorization", token.clone());
        Ok(req)
    });

    let point_request = Request::new(Point {
        latitude: 409146138,
        longitude: -746188906,
    });

    let point_response = route_guide_client.get_feature(point_request).await?;

    println!("RESPONSE={:?}", point_response);

    println!("\n*** SERVER STREAMING ***");
    print_features(&mut route_guide_client).await?;

    println!("\n*** CLIENT STREAMING ***");
    run_record_route(&mut route_guide_client).await?;

    println!("\n*** BIDIRECTIONAL STREAMING ***");
    run_route_chat(&mut route_guide_client).await?;

    

    Ok(())
}

fn random_point(rng: &mut ThreadRng) -> Point {
    let latitude = (rng.gen_range(0..180) - 90) * 10_000_000;
    let longitude = (rng.gen_range(0..360) - 180) * 10_000_000;
    Point {
        latitude,
        longitude,
    }
}

/// This function will get called on each outbound request. Returning a
/// `Status` here will cancel the request and have that status returned to
/// the caller.
fn intercept(req: Request<()>) -> Result<Request<()>, Status> {
    println!("Intercepting request: {:?}", req);
    Ok(req)
}

// You can also use the `Interceptor` trait to create an interceptor type
// that is easy to name
struct MyInterceptor;

impl Interceptor for MyInterceptor {
    fn call(&mut self, request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        Ok(request)
    }
}

#[allow(dead_code, unused_variables)]
async fn using_named_interceptor() -> Result<(), Box<dyn std::error::Error>> {
    let channel = Endpoint::from_static("http://[::1]:50051")
        .connect()
        .await?;

    let client: GreeterClient<InterceptedService<Channel, MyInterceptor>> =
        GreeterClient::with_interceptor(channel, MyInterceptor);

    Ok(())
}

// Using a function pointer type might also be possible if your interceptor is a
// bare function that doesn't capture any variables
#[allow(dead_code, unused_variables, clippy::type_complexity)]
async fn using_function_pointer_interceptro() -> Result<(), Box<dyn std::error::Error>> {
    let channel = Endpoint::from_static("http://[::1]:50051")
        .connect()
        .await?;

    let client: GreeterClient<
        InterceptedService<Channel, fn(tonic::Request<()>) -> Result<tonic::Request<()>, Status>>,
    > = GreeterClient::with_interceptor(channel, intercept);

    Ok(())
}