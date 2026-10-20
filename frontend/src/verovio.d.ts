declare module 'verovio/wasm' {
  const createVerovioModule: () => Promise<unknown>
  export default createVerovioModule
}
declare module 'verovio/esm' {
  export interface TimemapEntry { tstamp: number; qstamp: number; on?: string[]; off?: string[] }
  export class VerovioToolkit {
    constructor(module: unknown)
    setOptions(options: Record<string, unknown>): void
    loadData(data: string): boolean
    getPageCount(): number
    renderToSVG(page?: number): string
    renderToTimemap(options?: Record<string, unknown>): TimemapEntry[]
    getLog(): string
    destroy(): void
  }
}
