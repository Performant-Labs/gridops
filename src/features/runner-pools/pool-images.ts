type Provider = "docker" | "tart";

/**
 * The images a pool form sends: none for a provider the pool does not use
 * (issue #103), so a default typed into or prefilled for a provider that was
 * taken out again is never stored and later cloned. `image` is the primary
 * provider's image, as the API expects.
 */
export function poolImages({ provider, providers, dockerImage, tartImage }: {
  provider: Provider;
  providers: Provider[];
  dockerImage: string;
  tartImage: string;
}) {
  const docker = providers.includes("docker") ? dockerImage : "";
  const tart = providers.includes("tart") ? tartImage : "";
  return { image: provider === "tart" ? tart : docker, dockerImage: docker, tartImage: tart };
}

/** The settings form's starting image for a provider: the pool's own when it uses that provider, empty when adding it. */
export function startingImage(poolProviders: Provider[], kind: Provider, stored: string) {
  return poolProviders.includes(kind) ? stored : "";
}
