<?php

namespace App\Controller;

use App\Entity\User;
use App\Service\UserService;

interface Printable
{
    public function asString(): string;
}

interface Cloneable extends Printable, \JsonSerializable
{
    public function copy(): self;
}

trait LoggerTrait
{
    public function log(string $msg): void
    {
    }
}

trait CacheTrait
{
    public function cache(string $key): void
    {
    }
}

class AdminController extends \App\Foundation\Kernel implements Printable, Cloneable
{
    use LoggerTrait, CacheTrait;

    public function asString(): string
    {
        return 'admin';
    }

    public function copy(): self
    {
        return new self();
    }

    public function jsonSerialize(): array
    {
        return [];
    }
}
