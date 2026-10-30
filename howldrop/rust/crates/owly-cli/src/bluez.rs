//! The AirDrop BLE advertisement over BlueZ's LE advertising API
//! (BLE-1–3, BLE-5), for `owly-airdrop`'s sender.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

use owly_airdrop::proto::ble::{APPLE_COMPANY_ID, Advert, INTERVAL_MS};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const MANAGER: &str = "org.bluez.LEAdvertisingManager1";
const ADAPTER: &str = "org.bluez.Adapter1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// Advertises through the first adapter BlueZ offers LE advertising on.
pub struct BluezAdvertiser {
    conn: zbus::Connection,
    adapter: OwnedObjectPath,
}

impl BluezAdvertiser {
    pub async fn new() -> io::Result<Self> {
        let conn = zbus::Connection::system().await.map_err(io::Error::other)?;
        let adapter = find_adapter(&conn).await?;
        Ok(BluezAdvertiser { conn, adapter })
    }
}

async fn find_adapter(conn: &zbus::Connection) -> io::Result<OwnedObjectPath> {
    let objects = zbus::fdo::ObjectManagerProxy::builder(conn)
        .destination("org.bluez")
        .and_then(|b| b.path("/"))
        .map_err(io::Error::other)?
        .build()
        .await
        .map_err(io::Error::other)?
        .get_managed_objects()
        .await
        .map_err(io::Error::other)?;
    let mut adapters: Vec<_> = objects
        .into_iter()
        .filter(|(_, interfaces)| interfaces.keys().any(|i| i.as_str() == MANAGER))
        .map(|(path, _)| path)
        .collect();
    adapters.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    adapters.into_iter().next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no Bluetooth adapter with LE advertising",
        )
    })
}

/// The `org.bluez.LEAdvertisement1` object BlueZ reads the advert from.
struct Advertisement {
    data: Vec<u8>,
}

#[zbus::interface(name = "org.bluez.LEAdvertisement1")]
impl Advertisement {
    /// BlueZ dropped the advert, e.g. because the adapter went away.
    fn release(&self) {}

    /// BLE-5: connectable, as Apple's own adverts are, though nothing
    /// connects to it.
    #[zbus(property, name = "Type")]
    fn kind(&self) -> &str {
        "peripheral"
    }

    /// BLE-5: the interval current Apple devices answer to.
    #[zbus(property)]
    fn min_interval(&self) -> u32 {
        INTERVAL_MS
    }

    #[zbus(property)]
    fn max_interval(&self) -> u32 {
        INTERVAL_MS
    }

    /// BLE-1: Apple's company ID, holding the AirDrop TLV.
    #[zbus(property)]
    fn manufacturer_data(&self) -> HashMap<u16, Value<'_>> {
        HashMap::from([(APPLE_COMPANY_ID, Value::from(self.data.clone()))])
    }
}

static NEXT: AtomicU32 = AtomicU32::new(0);

impl BluezAdvertiser {
    /// Transmits `advert` until the registration is dropped.
    pub async fn advertise(&self, advert: Advert) -> io::Result<Registration> {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = ObjectPath::try_from(format!(
            "/io/github/t4t5/Owly/advert{}_{n}",
            std::process::id()
        ))
        .map_err(io::Error::other)?
        .into();
        let advertisement = Advertisement {
            data: advert.manufacturer_data(),
        };
        self.conn
            .object_server()
            .at(&path, advertisement)
            .await
            .map_err(io::Error::other)?;
        let mut link = Link {
            conn: self.conn.clone(),
            adapter: self.adapter.clone(),
            path,
            powered_on: false,
        };
        if let Err(e) = link.power_on().await {
            link.unregister().await;
            return Err(e);
        }
        if let Err(e) = link.register().await {
            link.unregister().await;
            return Err(e);
        }
        Ok(Registration(Some(link)))
    }
}

/// A registered advert's object, and where it's registered.
struct Link {
    conn: zbus::Connection,
    adapter: OwnedObjectPath,
    path: OwnedObjectPath,
    /// The adapter was off and this turned it on, so it's turned off again
    /// with the advert.
    powered_on: bool,
}

impl Link {
    /// BlueZ only advertises on a powered adapter.
    async fn power_on(&mut self) -> io::Result<()> {
        let reply = self
            .conn
            .call_method(
                Some("org.bluez"),
                self.adapter.as_str(),
                Some(PROPERTIES),
                "Get",
                &(ADAPTER, "Powered"),
            )
            .await
            .map_err(io::Error::other)?;
        let powered: OwnedValue = reply.body().deserialize().map_err(io::Error::other)?;
        if bool::try_from(powered).unwrap_or(false) {
            return Ok(());
        }
        self.set_powered(true).await.map_err(|e| {
            io::Error::other(format!("Bluetooth is off and couldn't be turned on: {e}"))
        })?;
        self.powered_on = true;
        Ok(())
    }

    async fn set_powered(&self, on: bool) -> zbus::Result<()> {
        self.conn
            .call_method(
                Some("org.bluez"),
                self.adapter.as_str(),
                Some(PROPERTIES),
                "Set",
                &(ADAPTER, "Powered", Value::from(on)),
            )
            .await
            .map(|_| ())
    }
    async fn register(&self) -> io::Result<()> {
        let options = HashMap::<&str, Value>::new();
        self.conn
            .call_method(
                Some("org.bluez"),
                self.adapter.as_str(),
                Some(MANAGER),
                "RegisterAdvertisement",
                &(&self.path, options),
            )
            .await
            .map(|_| ())
            .map_err(io::Error::other)
    }

    async fn unregister(self) {
        let _ = self
            .conn
            .call_method(
                Some("org.bluez"),
                self.adapter.as_str(),
                Some(MANAGER),
                "UnregisterAdvertisement",
                &(&self.path,),
            )
            .await;
        let _ = self
            .conn
            .object_server()
            .remove::<Advertisement, _>(&self.path)
            .await;
        if self.powered_on {
            let _ = self.set_powered(false).await;
        }
    }
}

/// Unregisters the advert when dropped.
pub struct Registration(Option<Link>);

impl Drop for Registration {
    fn drop(&mut self) {
        if let (Some(link), Ok(runtime)) = (self.0.take(), tokio::runtime::Handle::try_current()) {
            runtime.spawn(link.unregister());
        }
    }
}
