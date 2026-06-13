---
title: Blog
layout: default
nav_order: 5
permalink: /blog
description: "Notes from building Bifrost — progress, design decisions, and how to use it."
---

# Blog
{: .fs-9 }

Notes from building Bifrost — what we shipped, what we learned, and how to put it to
work on a real migration.
{: .fs-6 .fw-300 }

---

{% assign posts = site.posts %}
{% if posts.size == 0 %}
No posts yet.
{% else %}
{% for post in posts %}
### [{{ post.title }}]({{ post.url | relative_url }})

<p class="fs-3 fw-300 text-grey-dk-000">
{{ post.date | date: "%B %-d, %Y" }}{% if post.description %} &middot; {{ post.description }}{% endif %}
</p>

{% endfor %}
{% endif %}
