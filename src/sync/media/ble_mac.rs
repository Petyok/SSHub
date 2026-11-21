//! macOS transport of the Bluetooth channel: a CoreBluetooth peripheral manager through the objc2
//! bindings. The manager and its delegate live on a private serial dispatch queue; everything that
//! touches them runs there, the driver in `ble.rs` talks to it over `mpsc` and `exec_async`.
//!
//! The first use makes macOS ask the user for Bluetooth access for the terminal app. CoreBluetooth
//! offers a peripheral no way to disconnect a central: "drop" forgets the central (its writes are
//! ignored, nothing is notified to it) and the terminal advertises again; the central itself leaves
//! when its page does (contract C20).
//!
//! Type-checked in CI for the macOS targets only (no Mac in the loop): the first run on a Mac is the
//! real test.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, Message};
use objc2_core_bluetooth::{
    CBATTError, CBATTRequest, CBAdvertisementDataLocalNameKey, CBAdvertisementDataServiceUUIDsKey,
    CBAttributePermissions, CBCentral, CBCharacteristic, CBCharacteristicProperties, CBManager,
    CBManagerAuthorization, CBManagerState, CBMutableCharacteristic, CBMutableService,
    CBPeripheralManager, CBPeripheralManagerDelegate, CBService, CBUUID,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString,
};

use super::ble::{Poll, Transport};
use super::ble_link::{info_bytes, local_name, INFO_UUID, RX_UUID, SERVICE_UUID, TX_UUID};

/// Bluetooth side to the driver.
enum Up {
    Connected(usize),
    Message(Vec<u8>),
    Disconnected,
    Failed(String),
}

struct Ivars {
    up: Sender<Up>,
    connected: Arc<AtomicBool>,
    name: String,
    info: Vec<u8>,
    want_advertising: Cell<bool>,
    ready: Cell<bool>,
    tx_char: RefCell<Option<Retained<CBMutableCharacteristic>>>,
    central: RefCell<Option<Retained<CBCentral>>>,
    backlog: RefCell<VecDeque<Vec<u8>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` has no Drop.
    #[unsafe(super(NSObject))]
    #[name = "SshubBleDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBPeripheralManagerDelegate for Delegate {
        #[unsafe(method(peripheralManagerDidUpdateState:))]
        fn did_update_state(&self, peripheral: &CBPeripheralManager) {
            let state = unsafe { peripheral.state() };
            if state == CBManagerState::PoweredOn {
                self.publish(peripheral);
            } else if state == CBManagerState::Unauthorized {
                self.fail("Bluetooth access was refused: allow this terminal under System Settings, Privacy and Security, Bluetooth");
            } else if state == CBManagerState::PoweredOff {
                self.fail("Bluetooth is turned off");
            } else if state == CBManagerState::Unsupported {
                self.fail("this Mac cannot act as a Bluetooth LE peripheral");
            }
        }

        #[unsafe(method(peripheralManager:didAddService:error:))]
        fn did_add_service(
            &self,
            peripheral: &CBPeripheralManager,
            _service: &CBService,
            error: Option<&NSError>,
        ) {
            if let Some(e) = error {
                self.fail(&format!(
                    "cannot publish the GATT service: {}",
                    e.localizedDescription()
                ));
                return;
            }
            self.ivars().ready.set(true);
            if self.ivars().want_advertising.get() {
                self.advertise(peripheral);
            }
        }

        #[unsafe(method(peripheralManagerDidStartAdvertising:error:))]
        fn did_start_advertising(
            &self,
            _peripheral: &CBPeripheralManager,
            error: Option<&NSError>,
        ) {
            if let Some(e) = error {
                self.fail(&format!("cannot advertise: {}", e.localizedDescription()));
            }
        }

        #[unsafe(method(peripheralManager:central:didSubscribeToCharacteristic:))]
        fn did_subscribe(
            &self,
            _peripheral: &CBPeripheralManager,
            central: &CBCentral,
            characteristic: &CBCharacteristic,
        ) {
            if !is_uuid(characteristic, TX_UUID) || self.ivars().central.borrow().is_some() {
                return;
            }
            let cap = unsafe { central.maximumUpdateValueLength() };
            *self.ivars().central.borrow_mut() = Some(central.retain());
            self.ivars().backlog.borrow_mut().clear();
            self.ivars().connected.store(true, Ordering::SeqCst);
            let _ = self.ivars().up.send(Up::Connected(cap));
        }

        #[unsafe(method(peripheralManager:central:didUnsubscribeFromCharacteristic:))]
        fn did_unsubscribe(
            &self,
            _peripheral: &CBPeripheralManager,
            central: &CBCentral,
            characteristic: &CBCharacteristic,
        ) {
            if !is_uuid(characteristic, TX_UUID) {
                return;
            }
            let ours = self
                .ivars()
                .central
                .borrow()
                .as_ref()
                .is_some_and(|c| unsafe { c.identifier() == central.identifier() });
            if ours {
                *self.ivars().central.borrow_mut() = None;
                self.ivars().connected.store(false, Ordering::SeqCst);
                let _ = self.ivars().up.send(Up::Disconnected);
            }
        }

        #[unsafe(method(peripheralManager:didReceiveWriteRequests:))]
        fn did_receive_writes(
            &self,
            peripheral: &CBPeripheralManager,
            requests: &NSArray<CBATTRequest>,
        ) {
            let listening = self.ivars().central.borrow().is_some();
            for i in 0..requests.count() {
                let request = requests.objectAtIndex(i);
                let rx = unsafe { request.characteristic() };
                if listening && is_uuid(&rx, RX_UUID) {
                    if let Some(value) = unsafe { request.value() } {
                        let _ = self.ivars().up.send(Up::Message(value.to_vec()));
                    }
                }
            }
            if requests.count() > 0 {
                let first = requests.objectAtIndex(0);
                unsafe { peripheral.respondToRequest_withResult(&first, CBATTError::Success) };
            }
        }

        #[unsafe(method(peripheralManagerIsReadyToUpdateSubscribers:))]
        fn ready_to_update(&self, peripheral: &CBPeripheralManager) {
            self.flush(peripheral);
        }
    }
);

fn is_uuid(characteristic: &CBCharacteristic, uuid: &str) -> bool {
    let id = unsafe { characteristic.UUID().UUIDString() };
    id.to_string().eq_ignore_ascii_case(uuid)
}

fn cbuuid(s: &str) -> Retained<CBUUID> {
    unsafe { CBUUID::UUIDWithString(&NSString::from_str(s)) }
}

impl Delegate {
    fn new(ivars: Ivars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    fn fail(&self, why: &str) {
        let _ = self.ivars().up.send(Up::Failed(why.to_string()));
    }

    /// Powered on: publish the service (once).
    fn publish(&self, peripheral: &CBPeripheralManager) {
        if self.ivars().ready.get() || self.ivars().tx_char.borrow().is_some() {
            return;
        }
        let characteristic = |uuid: &str, props, value: Option<&NSData>, perms| unsafe {
            CBMutableCharacteristic::initWithType_properties_value_permissions(
                CBMutableCharacteristic::alloc(),
                &cbuuid(uuid),
                props,
                value,
                perms,
            )
        };
        let rx = characteristic(
            RX_UUID,
            CBCharacteristicProperties::Write | CBCharacteristicProperties::WriteWithoutResponse,
            None,
            CBAttributePermissions::Writeable,
        );
        let tx = characteristic(
            TX_UUID,
            CBCharacteristicProperties::Notify,
            None,
            CBAttributePermissions::Readable,
        );
        let info = NSData::with_bytes(&self.ivars().info);
        let info = characteristic(
            INFO_UUID,
            CBCharacteristicProperties::Read,
            Some(&info),
            CBAttributePermissions::Readable,
        );
        let chars: Retained<NSArray<CBCharacteristic>> = NSArray::from_retained_slice(&[
            Retained::into_super(rx.clone()),
            Retained::into_super(tx.clone()),
            Retained::into_super(info),
        ]);
        let service = unsafe {
            CBMutableService::initWithType_primary(
                CBMutableService::alloc(),
                &cbuuid(SERVICE_UUID),
                true,
            )
        };
        unsafe {
            service.setCharacteristics(Some(&chars));
            *self.ivars().tx_char.borrow_mut() = Some(tx);
            peripheral.addService(&service);
        }
    }

    fn advertise(&self, peripheral: &CBPeripheralManager) {
        let name = NSString::from_str(&self.ivars().name);
        let uuids = NSArray::from_retained_slice(&[cbuuid(SERVICE_UUID)]);
        let name_obj: &AnyObject = name.as_ref();
        let uuids_obj: &AnyObject = uuids.as_ref();
        let dict: Retained<NSDictionary<NSString, AnyObject>> = unsafe {
            NSDictionary::from_slices(
                &[
                    CBAdvertisementDataLocalNameKey,
                    CBAdvertisementDataServiceUUIDsKey,
                ],
                &[name_obj, uuids_obj],
            )
        };
        unsafe { peripheral.startAdvertising(Some(&dict)) };
    }

    /// Notifications queued while the transmit queue was full go out first.
    fn flush(&self, peripheral: &CBPeripheralManager) {
        let (Some(tx), Some(central)) = (
            self.ivars().tx_char.borrow().clone(),
            self.ivars().central.borrow().clone(),
        ) else {
            return;
        };
        let targets = NSArray::from_retained_slice(&[central]);
        loop {
            let Some(m) = self.ivars().backlog.borrow_mut().pop_front() else {
                break;
            };
            let data = NSData::with_bytes(&m);
            let sent = unsafe {
                peripheral.updateValue_forCharacteristic_onSubscribedCentrals(
                    &data,
                    &tx,
                    Some(&targets),
                )
            };
            if !sent {
                // wait for `peripheralManagerIsReadyToUpdateSubscribers:`
                self.ivars().backlog.borrow_mut().push_front(m);
                break;
            }
        }
    }
}

/// An Objective-C object that is only ever touched on the transport's dispatch queue.
struct OnQueue<T>(T);
// SAFETY: every access goes through `DispatchQueue::exec_*` on the one serial queue; the wrapper only
// moves the reference counts between threads (the `Arc` is shared, never dereferenced off the queue).
unsafe impl<T> Send for OnQueue<T> {}
unsafe impl<T> Sync for OnQueue<T> {}
impl<T> OnQueue<T> {
    fn get(&self) -> &T {
        &self.0
    }
}

pub(super) struct MacTransport {
    up: Receiver<Up>,
    queue: DispatchRetained<DispatchQueue>,
    state: Arc<OnQueue<(Retained<CBPeripheralManager>, Retained<Delegate>)>>,
    connected: Arc<AtomicBool>,
}

impl MacTransport {
    pub(super) fn start(tag: &str) -> MacTransport {
        let (up_tx, up) = channel();
        let connected = Arc::new(AtomicBool::new(false));
        let queue = DispatchQueue::new("sshub.bluetooth", None);
        let delegate = Delegate::new(Ivars {
            up: up_tx,
            connected: connected.clone(),
            name: local_name(tag),
            info: info_bytes(tag),
            want_advertising: Cell::new(false),
            ready: Cell::new(false),
            tx_char: RefCell::new(None),
            central: RefCell::new(None),
            backlog: RefCell::new(VecDeque::new()),
        });
        // the manager reports its state on `queue`, which starts the publication
        let manager = unsafe {
            CBPeripheralManager::initWithDelegate_queue(
                CBPeripheralManager::alloc(),
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            )
        };
        MacTransport {
            up,
            queue,
            state: Arc::new(OnQueue((manager, delegate))),
            connected,
        }
    }

    fn on_queue(&self, f: impl FnOnce(&CBPeripheralManager, &Delegate) + Send + 'static) {
        let state = self.state.clone();
        self.queue.exec_async(move || {
            let (manager, delegate) = state.get();
            f(manager, delegate);
        });
    }
}

impl Transport for MacTransport {
    fn poll(&mut self, wait: Duration) -> Poll {
        match self.up.recv_timeout(wait) {
            Ok(Up::Connected(cap)) => Poll::Connected { notify_cap: cap },
            Ok(Up::Message(m)) => Poll::Message(m),
            Ok(Up::Disconnected) => Poll::Disconnected,
            Ok(Up::Failed(why)) => Poll::Failed(why),
            Err(RecvTimeoutError::Timeout) => Poll::Idle,
            Err(RecvTimeoutError::Disconnected) => {
                Poll::Failed("the Bluetooth delegate ended".into())
            }
        }
    }

    fn send(&mut self, msg: &[u8]) -> bool {
        if !self.connected.load(Ordering::SeqCst) {
            return false;
        }
        let msg = msg.to_vec();
        self.on_queue(move |manager, delegate| {
            delegate.ivars().backlog.borrow_mut().push_back(msg);
            delegate.flush(manager);
        });
        true
    }

    fn advertise(&mut self, on: bool) {
        self.on_queue(move |manager, delegate| {
            delegate.ivars().want_advertising.set(on);
            if on {
                if delegate.ivars().ready.get() {
                    delegate.advertise(manager);
                }
            } else {
                unsafe { manager.stopAdvertising() };
            }
        });
    }

    fn disconnect(&mut self) {
        self.connected.store(false, Ordering::SeqCst);
        self.on_queue(|_, delegate| {
            *delegate.ivars().central.borrow_mut() = None;
            delegate.ivars().backlog.borrow_mut().clear();
        });
    }
}

impl Drop for MacTransport {
    fn drop(&mut self) {
        let state = self.state.clone();
        self.queue.exec_sync(move || {
            let (manager, _) = state.get();
            unsafe {
                manager.stopAdvertising();
                manager.removeAllServices();
                manager.setDelegate(None);
            }
        });
    }
}

/// Bluetooth access is not refused (the first real use asks the user).
pub(super) fn present() -> bool {
    let auth = unsafe { CBManager::authorization_class() };
    auth != CBManagerAuthorization::Denied && auth != CBManagerAuthorization::Restricted
}

pub(super) fn devices() -> Vec<String> {
    vec!["CoreBluetooth peripheral manager".to_string()]
}
