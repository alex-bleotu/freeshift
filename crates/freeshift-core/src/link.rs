use std::time::Duration;

use anyhow::{Result, bail};
use btleplug::api::BDAddr;
use tokio::time::{Instant, sleep};
use windows::Devices::Bluetooth::GenericAttributeProfile::GattSession;
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothLEDevice};

const POLL: Duration = Duration::from_millis(100);

pub struct Link {
    device: BluetoothLEDevice,
    session: GattSession,
}

impl Link {
    pub async fn open(address: BDAddr, wait: Duration) -> Result<Link> {
        let device = BluetoothLEDevice::FromBluetoothAddressAsync(u64::from(address))?.await?;
        let session = GattSession::FromDeviceIdAsync(&device.BluetoothDeviceId()?)?.await?;
        session.SetMaintainConnection(true)?;
        let link = Link { device, session };
        let deadline = Instant::now() + wait;
        while link.device.ConnectionStatus()? != BluetoothConnectionStatus::Connected {
            if Instant::now() >= deadline {
                bail!("the controller did not accept the link");
            }
            sleep(POLL).await;
        }
        Ok(link)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.session.SetMaintainConnection(false);
        let _ = self.session.Close();
        let _ = self.device.Close();
    }
}
