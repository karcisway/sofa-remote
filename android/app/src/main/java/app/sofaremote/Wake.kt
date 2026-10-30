package app.sofaremote

import org.json.JSONArray
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress

/** Wake-on-LAN: the "magic packet" that a sleeping PC's network adapter waits for. The PC sent its
 *  addresses while it was awake (`[{"mac": "aa:bb:..", "bcast": "192.168.1.255"}]`). */
object Wake {
    fun packet(mac: String): ByteArray? {
        val bytes = mac.split(':', '-').mapNotNull { it.toIntOrNull(16)?.toByte() }
        if (bytes.size != 6) return null
        return ByteArray(6) { -1 } + (1..16).flatMap { bytes }.toByteArray()
    }

    fun hasTargets(json: String?) = runCatching { JSONArray(json).length() > 0 }.getOrDefault(false)

    /** Blocking (network): call off the main thread. Returns how many packets went out. */
    fun send(json: String?): Int {
        val list = runCatching { JSONArray(json) }.getOrNull() ?: return 0
        var sent = 0
        DatagramSocket().use { socket ->
            socket.broadcast = true
            for (i in 0 until list.length()) {
                val target = list.optJSONObject(i) ?: continue
                val data = packet(target.optString("mac")) ?: continue
                // The network's own broadcast address, and the general one
                for (host in listOf(target.optString("bcast"), "255.255.255.255").filter { it.isNotEmpty() }) {
                    for (port in intArrayOf(9, 7)) {
                        runCatching {
                            socket.send(DatagramPacket(data, data.size, InetAddress.getByName(host), port))
                            sent++
                        }
                    }
                }
            }
        }
        return sent
    }
}
