/*
 * Copyright 2022 Google LLC
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

#[cfg(test)]
mod tests {
    use crate::{game_server, is_gameserver_ready, quilkin_config_map, quilkin_container, Client};
    use k8s_openapi::api::core::v1::{ConfigMap, ConfigMapVolumeSource, Volume};
    use kube::{api::PostParams, runtime::wait::await_condition, Api, ResourceExt};
    use quilkin::{config::watch::agones::crd::GameServer, test_utils::TestHelper};
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    /// This test exists to test that we can connect to an Agones GameServer directly.
    /// Useful in case one is not sure if an issue is Quilkin or general connectivity issues, such
    /// as a firewall settings, or an issue with Agones itself.
    async fn gameserver_no_sidecar() {
        let client = Client::new().await;
        let gameservers: Api<GameServer> = client.namespaced_api();

        let gs = game_server();

        let pp = PostParams::default();
        let gs = gameservers.create(&pp, &gs).await.unwrap();

        let name = gs.name();
        let ready = await_condition(gameservers.clone(), name.as_str(), is_gameserver_ready());
        timeout(Duration::from_secs(30), ready)
            .await
            .expect("GameServer should be ready")
            .unwrap();
        let gs = gameservers.get(name.as_str()).await.unwrap();

        let t = TestHelper::default();
        let recv = t.open_socket_and_recv_single_packet().await;
        let address = crate::gameserver_address(&gs);
        recv.socket
            .send_to("hello".as_bytes(), address)
            .await
            .unwrap();

        let response = timeout(Duration::from_secs(30), recv.packet_rx)
            .await
            .expect("should receive packet")
            .unwrap();
        assert_eq!("ACK: hello\n", response);
    }

    #[tokio::test]
    /// Testing Quilkin running as a sidecar next to a GameServer
    async fn gameserver_sidecar() {
        let client = Client::new().await;
        let config_maps: Api<ConfigMap> = client.namespaced_api();
        let gameservers: Api<GameServer> = client.namespaced_api();
        let pp = PostParams::default();

        // We'll append "sidecar", to prove the packet goes through the sidecar.
        let config = r#"
version: v1alpha1
filters:
  - name: quilkin.filters.concatenate_bytes.v1alpha1.ConcatenateBytes
    config:
        on_read: APPEND
        on_write: DO_NOTHING
        bytes: c2lkZWNhcg== # sidecar
clusters:
  default:
    localities:
      - endpoints:
          - address: 127.0.0.1:7654
"#;

        let config_map = config_maps
            .create(&pp, &quilkin_config_map(config))
            .await
            .unwrap();
        let mut gs = game_server();

        // reset ports to point at the Quilkin sidecar
        gs.spec.ports[0].container_port = 7000;
        gs.spec.ports[0].container = Some("quilkin".into());

        // set the gameserver container to the simple-game-server container.
        let mut template = gs.spec.template.spec.as_mut().unwrap();
        gs.spec.container = Some(template.containers[0].name.clone());

        let mount_name = "config".to_string();
        template
            .containers
            .push(quilkin_container(&client, Some(mount_name.clone())));

        template.volumes = Some(vec![Volume {
            name: mount_name,
            config_map: Some(ConfigMapVolumeSource {
                name: Some(config_map.name()),
                ..Default::default()
            }),
            ..Default::default()
        }]);

        let gs = gameservers.create(&pp, &gs).await.unwrap();
        let name = gs.name();
        let ready = await_condition(gameservers.clone(), name.as_str(), is_gameserver_ready());
        timeout(Duration::from_secs(30), ready)
            .await
            .expect("GameServer should be ready")
            .unwrap();
        let gs = gameservers.get(name.as_str()).await.unwrap();

        let t = TestHelper::default();
        let recv = t.open_socket_and_recv_single_packet().await;
        let address = crate::gameserver_address(&gs);
        recv.socket
            .send_to("hello".as_bytes(), address)
            .await
            .unwrap();

        let response = timeout(Duration::from_secs(30), recv.packet_rx)
            .await
            .expect("should receive packet")
            .unwrap();
        assert_eq!("ACK: hellosidecar\n", response);
    }
}
