use anyhow::Result;
use futures::{StreamExt, TryStreamExt, stream};
use std::future::Future;

pub async fn fanout_servers<T, F, Fut>(
    server_urls: Vec<String>,
    concurrency: usize,
    f: F,
) -> Result<Vec<(String, T)>>
where
    F: Fn(String) -> Fut + Copy,
    Fut: Future<Output = Result<Vec<T>>>,
{
    let results = stream::iter(server_urls)
        .map(|server_url| {
            let server_url_clone = server_url.clone();
            async move {
                let items = f(server_url).await?;
                Ok::<_, anyhow::Error>(
                    items
                        .into_iter()
                        .map(|item| (server_url_clone.clone(), item))
                        .collect::<Vec<_>>(),
                )
            }
        })
        .buffer_unordered(concurrency)
        .try_collect::<Vec<Vec<(String, T)>>>()
        .await?;

    Ok(results.into_iter().flatten().collect())
}

pub async fn find_on_servers<T, F, Fut>(
    server_urls: Vec<String>,
    concurrency: usize,
    f: F,
) -> Result<Option<(String, T)>>
where
    F: Fn(String) -> Fut + Copy,
    Fut: Future<Output = Result<Option<T>>>,
{
    let mut stream = stream::iter(server_urls)
        .map(|server_url| {
            let server_url_clone = server_url.clone();
            async move {
                let res = f(server_url).await?;
                Ok::<Option<(String, T)>, anyhow::Error>(res.map(|val| (server_url_clone, val)))
            }
        })
        .buffer_unordered(concurrency);

    while let Some(res) = stream.next().await {
        if let Some(found) = res? {
            return Ok(Some(found));
        }
    }

    Ok(None)
}
