---
title: "Why I Built Plexi"
date: 2026-07-01
description: "A terminal, an app runtime, and a place for agents to work beside you."
draft: true
---

I built Plexi because I wanted my computer to feel like mine again.

That sounds dramatic, but it is the shortest version of the story. Every app on your computer is someone else's idea of what you should be allowed to do. Sometimes that is good. A well-designed app can hide a lot of work. It can make the common path obvious.

But it also decides where the path ends.

The command line has the opposite problem. It does not decide much for you at all. That is why it feels hostile at first. It gives you almost no head start. But once you get used to it, the appeal is hard to unsee: the information and the action live in the same place.

You ask the computer for something. It answers. You act on the answer without moving to another window, another app, another mode.

That is the part I care about.

## The Trade

Computers are tools for making decisions.

You have some information. You choose what to do with it. Then you do it. That loop can be tiny, like renaming a file, or huge, like planning a project. Either way, the shape is the same: information in, action out.

Graphical apps are good at making specific decisions easier. Apple Notes is good at writing notes. Google Flights is good at comparing flights. Instagram is good at showing you what Instagram wants to show you.

The problem is that each app owns its little world. It decides what data you can see, how you can sort it, what buttons you get, and what workflows are worth supporting. If you want something slightly outside that shape, you wait for the developer to care.

The terminal does not work like that.

If the data is available, you can shape it. If an action can be expressed, you can run it. You can chain tools together. You can make your computer do weird, specific things that no product team would ever put on a roadmap.

That freedom is the whole point.

It is also why most people do not want to live there.

## Text Is Not Enough

I love the terminal. I also think terminal people lie to themselves a little.

Text is enough a lot of the time. Maybe most of the time. It is fast, searchable, scriptable, and honest. But some things are better with a real interface.

Graphs are better when you can see them. Games are better when they move. Forms are easier when the fields are right there. Dashboards can be useful when they are not pretending to be websites.

So the question became: what if the terminal did not have to stay text-only?

What if I could keep the part I loved, where information and action stay close together, but add the parts graphical apps are actually good at?

That is where Plexi started.

## What Plexi Is

Plexi is a terminal app with an app runtime inside it.

It has the normal terminal things: panes, splits, tabs, contexts, project roots. It also has UI apps that run beside terminal panes. It has agents that can work in the same space. It has a CLI that lets the host be driven from the shell.

The point is not to replace every app with a worse terminal version.

The point is to make a place where small tools can exist the moment you need them. A calculator. A todo list. A file viewer. A project dashboard. A tiny app an agent writes for one workflow and you keep because it fits how you think.

Those tools should not need a web server, an auth flow, a deploy, a subscription, and a tab in Chrome.

They should just open.

## Why Agents Matter

AI changed how I build software. It did not change where the work happens.

That became the problem. The agent is in one pane. The app is in another. The logs are somewhere else. The notes are in another app. The decisions are spread across the whole computer.

Plexi tries to pull that back into one surface.

An agent can help write an app. The app can run next to the terminal. The terminal can inspect the files. The host can manage panes, notifications, context, and workspace state. Everything is still local. Everything is still inspectable.

That matters because agents are only useful when they can see the work and act on it.

## Where It Is Now

Plexi is still young. I am building it in public, and parts of it are rough.

But I use it every day. Not as a demo. As my actual workspace.

That is why I am putting it out there now. I do not want to wait until the idea is sanded down into something boring. I want feedback while it still has the shape of the thing I meant to build.

If you want to try it, download the Mac app and run:

```sh
plexi demo
```

That is the smallest tour of the idea.

After that, split a pane, open a project, and see if the workspace starts to make sense.
