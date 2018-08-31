use std::{rc::Rc, cell::RefCell, net::SocketAddr};

use config::{Config, ConfigPtr};
use db::{
    Storage,
    Database,
    DatabasePtr,
    FileStorage,
    AccessoryList,
    AccessoryListMember,
    AccessoryListPtr,
};
use pin;
use protocol::Device;
use transport::{http, mdns::{Responder, ResponderPtr}, bonjour::StatusFlag, Transport};
use event::{Event, Emitter, EmitterPtr};

use Error;

/// Transport via TCP/IP.
pub struct IpTransport<S: Storage> {
    config: ConfigPtr,
    storage: S,
    database: DatabasePtr,
    accessories: AccessoryList,
    event_emitter: EmitterPtr,
    mdns_responder: ResponderPtr,
}

impl IpTransport<FileStorage> {
    /// Creates a new `IpTransport`.
    ///
    /// # Examples
    ///
    /// ```
    /// use hap::{
    ///     Config,
    ///     accessory::{Category, Information, bridge, lightbulb},
    ///     transport::{Transport, IpTransport},
    /// };
    ///
    /// let config = Config {
    ///     pin: "11122333".into(),
    ///     name: "Acme Lighting".into(),
    ///     category: Category::Bridge,
    ///     ..Default::default()
    /// };
    ///
    /// let bridge_info = Information {
    ///     name: "Bridge".into(),
    ///     ..Default::default()
    /// };
    /// let first_bulb_info = Information {
    ///     name: "Bulb 1".into(),
    ///     ..Default::default()
    /// };
    /// let second_bulb_info = Information {
    ///     name: "Bulb 2".into(),
    ///     ..Default::default()
    /// };
    ///
    /// let bridge = bridge::new(bridge_info).unwrap();
    /// let first_bulb = lightbulb::new(first_bulb_info).unwrap();
    /// let second_bulb = lightbulb::new(second_bulb_info).unwrap();
    ///
    /// let mut ip_transport = IpTransport::new(config).unwrap();
    /// ip_transport.add_accessory(bridge).unwrap();
    /// ip_transport.add_accessory(first_bulb).unwrap();
    /// ip_transport.add_accessory(second_bulb).unwrap();
    ///
    /// //ip_transport.start().unwrap();
    /// ```
    pub fn new(mut config: Config) -> Result<IpTransport<FileStorage>, Error> {
        let storage = FileStorage::new(&config.storage_path)?;
        let database = Database::new_with_file_storage(&config.storage_path)?;

        config.load_from(&storage)?;
        config.update_hash();
        config.save_to(&storage)?;

        let pin = pin::new(&config.pin)?;
        let device = Device::load_or_new(config.device_id.to_hex_string(), pin, &database)?;
        let event_emitter = Rc::new(RefCell::new(Emitter::new()));
        let mdns_responder = Rc::new(RefCell::new(Responder::new(&config.name, &config.port, config.txt_records())));

        let ip_transport = IpTransport {
            config: Rc::new(RefCell::new(config)),
            storage,
            database: Rc::new(RefCell::new(database)),
            accessories: AccessoryList::new(event_emitter.clone()),
            event_emitter,
            mdns_responder,
        };
        device.save_to(&ip_transport.database)?;

        Ok(ip_transport)
    }
}

impl Transport for IpTransport<FileStorage> {
    fn start(&mut self) -> Result<(), Error> {
        self.mdns_responder.try_borrow_mut()?.start();

        let (ip, port) = {
            let c = self.config.try_borrow()?;
            (c.ip, c.port)
        };

        let config = self.config.clone();
        let database = self.database.clone();
        let mdns_responder = self.mdns_responder.clone();
        self.event_emitter.try_borrow_mut()?.add_listener(Box::new(move |event| {
            match event {
                &Event::DevicePaired => {
                    match database.try_borrow()
                        .expect("couldn't access database")
                        .count_pairings() {
                        Ok(count) => if count > 0 {
                            let mut c = config.try_borrow_mut()
                                .expect("couldn't access config");
                            c.status_flag = StatusFlag::Zero;
                            mdns_responder.try_borrow_mut()
                                .expect("couldn't access mDNS responder")
                                .update_txt_records(c.txt_records())
                                .expect("couldn't update mDNS TXT records");
                        },
                        _ => {},
                    }
                },
                &Event::DeviceUnpaired => {
                    match database.try_borrow()
                        .expect("couldn't access database")
                        .count_pairings() {
                        Ok(count) => if count == 0 {
                            let mut c = config.try_borrow_mut()
                                .expect("couldn't access config");
                            c.status_flag = StatusFlag::NotPaired;
                            mdns_responder.try_borrow_mut()
                                .expect("couldn't access mDNS responder")
                                .update_txt_records(c.txt_records())
                                .expect("couldn't update mDNS TXT records");
                        },
                        _ => {},
                    }
                },
                _ => {},
            }
        }));

        http::server::serve(
            &SocketAddr::new(ip, port),
            self.config.clone(),
            self.database.clone(),
            self.accessories.clone(),
            self.event_emitter.clone(),
        )?;
        Ok(())
    }

    fn stop(&self) -> Result<(), Error> {
        self.mdns_responder.try_borrow()?.stop()?;
        Ok(())
    }

    fn add_accessory<A: 'static + AccessoryListMember>(&mut self, accessory: A) -> Result<AccessoryListPtr, Error> {
        self.accessories.add_accessory(Box::new(accessory))
    }

    fn remove_accessory(&mut self, accessory: &AccessoryListPtr) -> Result<(), Error> {
        self.accessories.remove_accessory(accessory)
    }

    // fn load_accessories(&mut self) -> Result<(), Error> {
    //     if let Some(device_id) = storage.get_bytes("device_id").ok() {
    //         self.device_id = MacAddress::parse_str(str::from_utf8(&device_id)?)?;
    //     }
    //     if let Some(version) = storage.get_u64("version").ok() {
    //         self.version = version;
    //     }
    //     if let Some(config_hash) = storage.get_u64("config_hash").ok() {
    //         self.config_hash = Some(config_hash);
    //     }
    //     Ok(())
    // }
    //
    // fn save_accessories(&self) -> Result<(), Error> {
    //     storage.set_bytes("accessories", self.accessories.as_bytes()?)
    // }
}
