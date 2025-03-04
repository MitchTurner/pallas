use pallas::network::{
    miniprotocols::{chainsync, handshake, localstate, Point, MAINNET_MAGIC},
    multiplexer,
};

#[derive(Debug)]
struct LoggingObserver;

#[allow(dead_code)]
fn do_handshake(channel: multiplexer::StdChannel) {
    let mut client = handshake::N2CClient::new(channel);

    let confirmation = client
        .handshake(handshake::n2c::VersionTable::v1_and_above(MAINNET_MAGIC))
        .unwrap();

    match confirmation {
        handshake::Confirmation::Accepted(v, _) => {
            log::info!("hand-shake accepted, using version {}", v)
        }
        handshake::Confirmation::Rejected(x) => {
            log::info!("hand-shake rejected with reason {:?}", x)
        }
    }
}

#[allow(dead_code)]
fn do_localstate_query(channel: multiplexer::StdChannel) {
    let mut client = localstate::ClientV10::new(channel);
    client.acquire(None).unwrap();

    let result = client
        .query(localstate::queries::RequestV10::GetSystemStart)
        .unwrap();

    log::info!("system start result: {:?}", result);
}

#[allow(dead_code)]
fn do_chainsync(channel: multiplexer::StdChannel) {
    let known_points = vec![Point::Specific(
        43847831u64,
        hex::decode("15b9eeee849dd6386d3770b0745e0450190f7560e5159b1b3ab13b14b2684a45").unwrap(),
    )];

    let mut client = chainsync::N2CClient::new(channel);

    let (point, _) = client.find_intersect(known_points).unwrap();

    log::info!("intersected point is {:?}", point);

    for _ in 0..10 {
        let next = client.request_next().unwrap();

        match next {
            chainsync::NextResponse::RollForward(h, _) => {
                log::info!("rolling forward, block size: {}", h.len())
            }
            chainsync::NextResponse::RollBackward(x, _) => log::info!("rollback to {:?}", x),
            chainsync::NextResponse::Await => log::info!("tip of chain reached"),
        };
    }
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .init();

    #[cfg(not(target_family = "unix"))]
    {
        panic!("can't use n2c unix socket on non-unix systems");
    }
    // we connect to the unix socket of the local node. Make sure you have the right
    // path for your environment
    #[cfg(target_family = "unix")]
    {
        use pallas::network::{
            miniprotocols::{
                PROTOCOL_N2C_CHAIN_SYNC, PROTOCOL_N2C_HANDSHAKE, PROTOCOL_N2C_STATE_QUERY,
            },
            multiplexer::bearers::Bearer,
        };
        let bearer = Bearer::connect_unix("/tmp/node.socket").unwrap();

        // setup the multiplexer by specifying the bearer and the IDs of the
        // miniprotocols to use
        let mut plexer = multiplexer::StdPlexer::new(bearer);
        let handshake = plexer.use_client_channel(PROTOCOL_N2C_HANDSHAKE);
        let statequery = plexer.use_client_channel(PROTOCOL_N2C_STATE_QUERY);
        let chainsync = plexer.use_client_channel(PROTOCOL_N2C_CHAIN_SYNC);

        plexer.muxer.spawn();
        plexer.demuxer.spawn();

        // execute the required handshake against the relay
        do_handshake(handshake);

        // execute an arbitrary "Local State" query against the node
        do_localstate_query(statequery);

        // execute the chainsync flow from an arbitrary point in the chain
        do_chainsync(chainsync);
    }
}
