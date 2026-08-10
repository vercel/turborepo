import Link from "next/link";
import Image from "next/image";

interface CardProps {
  name: string;
  alias: string;
  avatar: string;
  url: string;
  children: React.ReactNode;
}

export const Card = ({ name, alias, avatar, url, children }: CardProps) => {
  return (
    <Link
      href={url}
      target="_blank"
      className="flex flex-col gap-3 rounded-lg border border-gray-300 bg-background-100 p-4 transition-colors hover:border-gray-500 md:p-5"
    >
      <div className="flex items-center gap-2.5">
        <Image
          src={avatar}
          alt={name}
          width={40}
          height={40}
          className="size-9 rounded-full bg-gray-200"
        />
        <div className="flex flex-col">
          <div className="font-medium text-gray-1000 text-sm">{name}</div>
          <div className="text-gray-900 text-sm">{alias}</div>
        </div>
      </div>
      <div className="flex flex-col gap-2.5 text-gray-900 text-sm leading-relaxed [&_p]:m-0">
        {children}
      </div>
    </Link>
  );
};

export const Testimonials = () => {
  return (
    <div className="mb-6 mt-16 grid grid-cols-1 gap-4 min-[780px]:grid-cols-3">
      <div className="grid gap-4">
        <Card
          name="Matt Pocock"
          alias="@mattpocockuk"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/mattpocockuk.jpg"
          url="https://x.com/mattpocockuk/status/1498696992943452168"
        >
          <p className="mb-4">
            🤯 @turborepo saved us 67 HOURS of CI since we adopted it.
          </p>
          <p>That's a team of only 4 full-time devs at @statelyai. Nuts.</p>
        </Card>
        <Card
          name="Lewis ⚡"
          alias="@lewisbuildsai"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/lewisbuildsai.jpg"
          url="https://x.com/lewisbuildsai/status/1906014322926420104"
        >
          <p className="mb-4">
            If turborepo has a million fans, then I am one of them. If turborepo
            has ten fans, then I am one of them. If turborepo has only one fan
            then that is me. If turborepo has no fans, then that means I am no
            longer on earth. If the world is against turborepo, then I am
            against the world.
          </p>
        </Card>
      </div>

      <div className="grid gap-4">
        <Card
          name="Cory House"
          alias="@housecor"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/housecor.jpg"
          url="https://x.com/housecor/status/1580923528596946944"
        >
          <p className="mb-4">
            I'm really enjoying @turborepo. It's a game changer.
          </p>

          <p className="mb-4">
            With Turbo, I have an incentive to break my repo down into many
            small, focused packages. This way, Turbo can cache each little
            build. So when I hit save, only the package I changed needs to
            recompile.
          </p>
          <p>Really speeds things up!</p>
        </Card>
        <Card
          name="Shrey Gupta"
          alias="@shreygups"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/shreygups.jpg"
          url="https://x.com/shreygups/status/1900284978367520817"
        >
          <p>wait i kinda love turborepo</p>
        </Card>
        <Card
          name="Jono Alford"
          alias="@jonoalford"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/jonoalford.jpg"
          url="https://x.com/jonoalford/status/1989274609192169943?s=20"
        >
          <p>
            The second we started using this for monorepos is the second we
            stopped having to worry about using monorepos.
          </p>
        </Card>
      </div>
      <div className="grid gap-4">
        <Card
          name="ALIAS"
          alias="LoadingALIAS"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/loadingalias.jpg"
          url="https://x.com/LoadingALIAS/status/1903228609021477283"
        >
          <p>Turborepo is the best.</p>
        </Card>
        <Card
          name="Andrew Lisowski"
          alias="@HipsterSmoothie"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/hipstersmoothie.jpg"
          url="https://x.com/HipsterSmoothie/status/1829245704386269515"
        >
          <p>
            Just setting up @turborepo in @DescriptApp's front-end cut our bill
            in half and saved us $20k
          </p>
        </Card>
        <Card
          name="Pontus Abrahamsson"
          alias="@pontusab"
          avatar="https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/user-avatars/pontusab.jpg"
          url="https://x.com/pontusab/status/1827264818765799931"
        >
          <p>
            I love a monorepo setup, having everything in one place powered by
            Turborepo 🔥
          </p>
        </Card>
      </div>
    </div>
  );
};
