import type { Job } from '../types'

export class ApiError extends Error {}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const r = await fetch(url, init)
  if (!r.ok) {
    let msg = r.statusText
    try {
      const j = await r.json()
      msg = typeof j.detail === 'string' ? j.detail : JSON.stringify(j.detail)
    } catch { /* not json */ }
    throw new ApiError(msg)
  }
  return r.json() as Promise<T>
}

export const get = <T,>(url: string, signal?: AbortSignal) => request<T>(url, { signal })
export const post = <T,>(url: string, body: unknown, signal?: AbortSignal) =>
  request<T>(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal })
export const del = <T,>(url: string) => request<T>(url, { method: 'DELETE' })

/** Poll a background job until it finishes. Rejects with the server's error message. */
export async function pollJob<T>(id: string, onUpdate: (j: Job<T>) => void, signal?: AbortSignal): Promise<T> {
  for (;;) {
    const j = await get<Job<T>>(`/api/job/${id}`, signal)
    onUpdate(j)
    if (j.state === 'error') throw Object.assign(new ApiError(j.error ?? 'Something went wrong'), { job: j })
    if (j.state === 'done') return j.result as T
    await new Promise((res) => setTimeout(res, 450))
  }
}

export const qs = (o: Record<string, string | number | boolean | null | undefined>) => {
  const p = new URLSearchParams()
  for (const [k, v] of Object.entries(o)) if (v !== null && v !== undefined && v !== '') p.set(k, String(v))
  return p.toString()
}
