package org.oneamp

/**
 * Resolves and renews the access to remote media. Called on OneAmp's
 * decode threads (never the main thread, never the audio thread); may
 * block on the network.
 *
 * Both methods return JSON: an access
 * `{"url": "...", "headers": {"Name": "value"}, "expiresAtMs": 0}`
 * or a refusal `{"error": {"category": "AccessExpired" | "AccessDenied", "message": "..."}}`.
 *
 * [renew] is called once per rejection (HTTP 401). It must return an
 * access with no more rights than the rejected one, and must never fall
 * back to a long-lived credential in a URL. Concurrent rejections of the
 * same access are coordinated by OneAmp: one renewal, shared.
 */
interface AccessCallback {
    fun access(mediaId: String): String
    fun renew(mediaId: String, reason: String, rejectedGeneration: Long): String
}
