use gasket::error::AsWorkError;
use tracing::{debug, info};

use pallas_miniprotocols::chainsync::{HeaderContent, NextResponse};
use pallas_miniprotocols::{chainsync, Point};
use pallas_traverse::MultiEraHeader;

use crate::cursor::{Cursor, Intersection};
use crate::framework::*;

fn to_traverse(header: &chainsync::HeaderContent) -> Result<MultiEraHeader<'_>, Error> {
    let out = match header.byron_prefix {
        Some((subtag, _)) => MultiEraHeader::decode(header.variant, Some(subtag), &header.cbor),
        None => MultiEraHeader::decode(header.variant, None, &header.cbor),
    };

    out.map_err(Error::parse)
}

pub type DownstreamPort = gasket::messaging::OutputPort<ChainSyncEvent>;

pub type OuroborosClient = chainsync::N2NClient<ProtocolChannel>;

pub struct Worker {
    chain_cursor: Cursor,
    client: OuroborosClient,
    downstream: DownstreamPort,
    block_count: gasket::metrics::Counter,
    chain_tip: gasket::metrics::Gauge,
}

impl Worker {
    pub fn new(chain_cursor: Cursor, plexer: ProtocolChannel, downstream: DownstreamPort) -> Self {
        let client = OuroborosClient::new(plexer);

        Self {
            chain_cursor,
            client,
            downstream,
            block_count: Default::default(),
            chain_tip: Default::default(),
        }
    }

    fn intersect(&mut self) -> Result<Option<Point>, gasket::error::Error> {
        let value = self.chain_cursor.read();

        match value {
            Intersection::Origin => {
                let point = self.client.intersect_origin().or_restart()?;

                Ok(Some(point))
            }
            Intersection::Tip => {
                let point = self.client.intersect_tip().or_restart()?;

                Ok(Some(point))
            }
            Intersection::Breadcrumbs(points) => {
                let (point, _) = self.client.find_intersect(Vec::from(points)).or_restart()?;

                Ok(point)
            }
        }
    }

    fn process_next(
        &mut self,
        next: NextResponse<HeaderContent>,
    ) -> Result<(), gasket::error::Error> {
        match next {
            chainsync::NextResponse::RollForward(h, t) => {
                let h = to_traverse(&h).or_panic()?;
                self.downstream
                    .send(ChainSyncEvent::RollForward(h.slot(), h.hash()).into())?;

                debug!(slot = h.slot(), hash = %h.hash(), "chain sync roll forward");
                self.chain_tip.set(t.1 as i64);
                Ok(())
            }
            chainsync::NextResponse::RollBackward(p, t) => {
                self.downstream.send(ChainSyncEvent::Rollback(p).into())?;
                self.chain_tip.set(t.1 as i64);
                Ok(())
            }
            chainsync::NextResponse::Await => {
                info!("chain-sync reached the tip of the chain");
                Ok(())
            }
        }
    }

    fn request_next(&mut self) -> Result<(), gasket::error::Error> {
        info!("requesting next block");
        let next = self.client.request_next().or_restart()?;
        self.process_next(next)
    }

    fn await_next(&mut self) -> Result<(), gasket::error::Error> {
        info!("awaiting next block (blocking)");
        let next = self.client.recv_while_must_reply().or_restart()?;
        self.process_next(next)
    }
}

impl gasket::runtime::Worker for Worker {
    fn metrics(&self) -> gasket::metrics::Registry {
        gasket::metrics::Builder::new()
            .with_counter("received_blocks", &self.block_count)
            .with_gauge("chain_tip", &self.chain_tip)
            .build()
    }

    fn bootstrap(&mut self) -> Result<(), gasket::error::Error> {
        let intersect = self.intersect()?;
        info!(?intersect, "chain-sync intersected");

        Ok(())
    }

    fn work(&mut self) -> gasket::runtime::WorkResult {
        match self.client.has_agency() {
            true => self.request_next()?,
            false => self.await_next()?,
        };

        Ok(gasket::runtime::WorkOutcome::Partial)
    }
}
